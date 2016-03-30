use std::cmp::min;

use AlacConfig;
use bitcursor::BitCursor;

pub trait Sample {
    /// Constructs `Self` from a right-aligned `bits` bit sample
    fn from_decoder(sample: i32, bits: u8) -> Self;

    fn bits() -> u8;
}

impl Sample for i16 {
    #[inline(always)]
    fn from_decoder(sample: i32, _: u8) -> Self {
        sample as i16
    }

    #[inline(always)]
    fn bits() -> u8 {
        16
    }
}

impl Sample for i32 {
    #[inline(always)]
    fn from_decoder(sample: i32, bits: u8) -> Self {
        sample << (32 - bits)
    }

    #[inline(always)]
    fn bits() -> u8 {
        32
    }
}

pub struct Decoder {
    config: AlacConfig,
    mix_buf_u: Vec<i32>,
    mix_buf_v: Vec<i32>,
}

const ID_SCE: u8 = 0; // Single Channel Element
const ID_CPE: u8 = 1; // Channel Pair Element
const ID_CCE: u8 = 2; // Coupling Channel Element
const ID_LFE: u8 = 3; // LFE Channel Element
const ID_DSE: u8 = 4; // not yet supported
const ID_PCE: u8 = 5;
const ID_FIL: u8 = 6; // filler element
const ID_END: u8 = 7; // frame end

impl Decoder {
    pub fn new(config: AlacConfig) -> Decoder {
        let new_buf = || vec![0; config.frame_length as usize];

        Decoder {
            config: config,
            mix_buf_u: new_buf(),
            mix_buf_v: new_buf(),
        }
    }

    pub fn config(&self) -> &AlacConfig {
        &self.config
    }

    pub fn decode_packet<'a, S: Sample>(&mut self,
                                        packet: &[u8],
                                        out: &'a mut [S])
                                        -> Result<&'a [S], ()> {
        let mut reader = BitCursor::new(packet);

        let mut channel_index = 0;
        let mut num_samples: u32 = self.config.frame_length;

        assert!(out.len() >= self.config.frame_length as usize * self.config.num_channels as usize);
        assert!(S::bits() >= self.config.bit_depth);

        loop {
            let tag = try!(reader.read_u8(3));

            match tag {
                tag @ ID_SCE | tag @ ID_LFE | tag @ ID_CPE => {
                    let packet_channels = match tag {
                        ID_SCE => 1,
                        ID_LFE => 1,
                        ID_CPE => 2,
                        _ => unreachable!(),
                    };

                    if channel_index + packet_channels > self.config.num_channels {
                        debug_assert!(false, "Too many channels");
                        return Err(()); // TOO MANY CHANNELS
                    }

                    num_samples = try!(decode_audio_element(self,
                                              &mut reader,
                                              out,
                                              channel_index,
                                              packet_channels));

                    channel_index += packet_channels;
                }
                ID_CCE | ID_PCE => {
                    // unsupported element, bail
                    return Err(());
                }
                ID_DSE => {
                    // data stream element -- parse but ignore

                    // the tag associates this data stream element with a given audio element
                    // Unused
                    let _element_instance_tag = try!(reader.read_u8(4));
                    let data_byte_align_flag = try!(reader.read_bit());

                    // 8-bit count or (8-bit + 8-bit count) if 8-bit count == 255
                    let mut skip_bytes = try!(reader.read_u8(8)) as usize;
                    if skip_bytes == 255 {
                        skip_bytes += try!(reader.read_u8(8)) as usize;
                    }

                    // the align flag means the bitstream should be byte-aligned before reading the following data bytes
                    if data_byte_align_flag {
                        try!(reader.skip_to_byte());
                    }

                    try!(reader.skip(skip_bytes * 8));
                }
                ID_FIL => {
                    // fill element -- parse but ignore

                    // 4-bit count or (4-bit + 8-bit count) if 4-bit count == 15
                    // - plus this weird -1 thing I still don't fully understand
                    let mut skip_bytes = try!(reader.read_u8(4)) as usize;
                    if skip_bytes == 15 {
                        skip_bytes += try!(reader.read_u8(8)) as usize - 1
                    }

                    try!(reader.skip(skip_bytes * 8));
                }
                ID_END => {
                    // frame end, all done so byte align the frame and check for overruns
                    try!(reader.skip_to_byte());

                    // TODO: Check no leftover buffer data (in debug only)

                    if channel_index != self.config.num_channels {
                        panic!("not enough channels");
                    }

                    return Ok((&out[..num_samples as usize * self.config.num_channels as usize]));
                }
                _ => unreachable!(), // 3 bit tag with all 8 options exhausted
            }
        }
    }
}

fn decode_audio_element<'a, S: Sample>(this: &mut Decoder,
                                       reader: &mut BitCursor<'a>,
                                       out: &mut [S],
                                       channel_index: u8,
                                       packet_channels: u8)
                                       -> Result<u32, ()> {
    // Unused
    let _element_instance_tag = try!(reader.read_u8(4));

    let unused = try!(reader.read_u16(12));
    if unused != 0 {
        return Err(()); // Unused header data not 0
    }

    // read the 1-bit "partial frame" flag, 2-bit "shift-off" flag & 1-bit "escape" flag
    let partial_frame = try!(reader.read_bit());

    let bytes_shifted = try!(reader.read_u8(2));
    if bytes_shifted >= 3 {
        return Err(()); // must be 1 or 2
    }

    let is_uncompressed = try!(reader.read_bit());

    // check for partial frame to override requested numSamples
    let num_samples = if partial_frame {
        // TODO: this could change within a frame. That would be bad
        let num_samples = try!(reader.read_u32(32));

        if num_samples > this.config.frame_length {
            return Err(());
        }

        num_samples as usize
    } else {
        this.config.frame_length as usize
    };

    if !is_uncompressed {
        // TODO: Treat as contiguous buffer?
        let mut mix_buf = [&mut this.mix_buf_u[..num_samples], &mut this.mix_buf_v[..num_samples]];

        let shift = bytes_shifted * 8;
        let chan_bits = this.config.bit_depth - shift + packet_channels - 1;

        // compressed frame, read rest of parameters
        let mix_bits: u8 = try!(reader.read_u8(8));
        let mix_res: i8 = try!(reader.read_u8(8)) as i8;

        let mut lpc_mode = [0; 2]; //u8
        let mut lpc_quant = [0; 2]; //u32
        let mut pb_factor = [0; 2]; //u16
        let mut lpc_order = [0; 2]; //u8
        let mut lpc_coefs = [[0; 32]; 2]; //i16*

        for i in 0..(packet_channels as usize) {
            lpc_mode[i] = try!(reader.read_u8(4));
            lpc_quant[i] = try!(reader.read_u8(4)) as u32;
            pb_factor[i] = try!(reader.read_u8(3)) as u16;
            lpc_order[i] = try!(reader.read_u8(5));

            // Coefficients are used in reverse order of storage for prediction
            for j in (0..lpc_order[i] as usize).rev() {
                lpc_coefs[i][j] = try!(reader.read_u16(16)) as i16;
            }
        }

        let extra_bits_reader = if bytes_shifted != 0 {
            let extra_bits_reader = reader.clone();
            try!(reader.skip((bytes_shifted as usize * 8) * num_samples *
                             packet_channels as usize));
            Some(extra_bits_reader)
        } else {
            None
        };

        // TODO: Tidy and comment these steps see below for an example
        // https://github.com/ruud-v-a/claxon/blob/master/src/subframe.rs
        // It should be possible to it without allocating buffers quite easily
        for i in 0..(packet_channels as usize) {
            try!(rice_decompress(reader, &this.config, &mut mix_buf[i], chan_bits, pb_factor[i]));

            if lpc_mode[i as usize] == 15 {
                // the special "numActive == 31" mode can be done in-place
                lpc_predict_order_31(mix_buf[i], chan_bits);
            } else if lpc_mode[i as usize] > 0 {
                return Err(());
            }

            // We have a seperate function for this
            assert!(lpc_order[i] != 31);

            let lpc_coefs = &mut lpc_coefs[i][..lpc_order[i] as usize];
            lpc_predict(mix_buf[i], chan_bits, lpc_coefs, lpc_quant[i]);
        }

        if packet_channels == 2 && mix_res != 0 {
            unmix_stereo(&mut mix_buf, mix_bits, mix_res);
        }

        // now read the shifted values into the shift buffer
        // We directly apply the shifts to avoid needing a buffer
        if let Some(mut extra_bits_reader) = extra_bits_reader {
            try!(append_extra_bits(&mut extra_bits_reader,
                                   &mut mix_buf,
                                   packet_channels,
                                   bytes_shifted));
        }

        for i in 0..num_samples {
            for j in 0..packet_channels as usize {
                let sample = mix_buf[j][i];

                let idx = i * this.config.num_channels as usize + channel_index as usize + j;

                out[idx] = S::from_decoder(sample, this.config.bit_depth);
            }
        }

    } else {
        // uncompressed frame, copy data into the mix buffers to use common output code

        // Here we deviate here from the reference implementation and just copy
        // straight to the output buffer.

        if bytes_shifted != 0 {
            return Err(());
        }

        for i in 0..num_samples {
            for j in 0..packet_channels as usize {
                let sample = try!(reader.read_u32(this.config.bit_depth as usize)) as i32;
                let sample = sign_extend(sample, this.config.bit_depth);

                let idx = i * this.config.num_channels as usize + channel_index as usize + j;

                out[idx] = S::from_decoder(sample, this.config.bit_depth);
            }
        }
    }

    Ok(num_samples as u32)
}

fn decode_rice_scalar<'a>(reader: &mut BitCursor<'a>, m: u32, k: u8, bps: u8) -> Result<u32, ()> {
    // Count the numder of leading 1s up to a maximum of 9
    let bits = try!(reader.peek_u16(9)) << 7;
    let mut x = (!bits).leading_zeros();
    // We want to skip the terminating bit as well if it exists
    try!(reader.skip(min(x as usize + 1, 9)));

    if x > 8 {
        x = try!(reader.read_u32(bps as usize));
    } else if k != 1 {
        let extrabits = try!(reader.peek_u32(k as usize));

        // TODO: Investigate the differences between these
        // x = (x << k) - x;
        x *= m;

        if extrabits > 1 {
            x += extrabits - 1;
            try!(reader.skip(k as usize));
        } else {
            try!(reader.skip(k as usize - 1));
        }
    }

    Ok(x)
}

fn rice_decompress<'a>(reader: &mut BitCursor<'a>,
                       config: &AlacConfig,
                       buf: &mut [i32],
                       chan_bits: u8,
                       pb_factor: u16)
                       -> Result<(), ()> {

    fn log_2(x: u32) -> u32 {
        31 - (x | 1).leading_zeros()
    }

    let mut history: u32 = config.mb as u32;
    let rice_limit = config.kb;
    let bps = chan_bits;
    let mut sign_modifier = 0;
    let rice_history_mult = (config.pb as u32 * pb_factor as u32) / 4; //pb
    let num_samples = buf.len();
    let wb_local = (1 << rice_limit) - 1;

    let mut i = 0;
    while i < num_samples {
        let k = log_2((history >> 9) + 3);
        let k = min(k as u8, rice_limit);
        let m = (1 << k) - 1;
        // TODO: check about the m thing
        let x = try!(decode_rice_scalar(reader, m, k, bps));
        let x = x + sign_modifier;
        sign_modifier = 0;
        buf[i] = ((x >> 1) as i32) ^ -((x & 1) as i32);

        // update the history
        if x > 0xffff {
            history = 0xffff;
        } else {
            // Avoid assignment add do we don't underflow
            history = (history + x * rice_history_mult) - ((history * rice_history_mult) >> 9);
        }

        // special case: there may be compressed blocks of 0
        if (history < 128) && (i + 1 < num_samples) {
            // calculate rice param and decode block size
            let k = 7 - log_2(history) + ((history + 16) >> 6);
            let k = min(k as u8, rice_limit);
            let mz = ((1 << k) - 1) & wb_local;
            let block_size = try!(decode_rice_scalar(reader, mz, k, 16)) as usize;

            if block_size > 0 {
                if block_size >= num_samples - i {
                    panic!("");
                    return Err(());
                    // FFMPEG continues here but the reference decoder does not
                }
                // TODO: memset
                for j in i + 1..i + 1 + block_size {
                    buf[j] = 0;
                }
                i += block_size;
            }
            if block_size <= 0xffff {
                sign_modifier = 1;
            }
            history = 0;
        }

        i += 1;
    }
    Ok(())
}

#[inline(always)]
fn sign_extend(val: i32, bits: u8) -> i32 {
    let shift = 32 - bits;
    (val << shift) >> shift
}

fn lpc_predict_order_31(buf: &mut [i32], bps: u8) {
    for i in 1..buf.len() {
        buf[i] = sign_extend(buf[i] + buf[i - 1], bps);
    }
}

fn lpc_predict(buf: &mut [i32], bps: u8, lpc_coefs: &mut [i16], lpc_quant: u32) {
    let lpc_order = lpc_coefs.len();

    // Read warm-up samples
    for i in 1..min(lpc_order + 1, buf.len()) {
        buf[i] = sign_extend(buf[i] + buf[i - 1], bps);
    }

    // TODO: Might be worth doing a couple of unrolled versions for order 4 and 8
    for i in (lpc_order + 1)..buf.len() {
        let d = buf[i - lpc_order - 1];
        let pred_index = i - lpc_order;
        let prediction_error = buf[i];

        // Do LPC prediction
        let mut val = 0;
        for j in 0..lpc_order {
            val += (buf[pred_index + j] - d) * (lpc_coefs[j] as i32);
        }
        // 1 << (lpc_quant - 1) sets the lpc_quant'th bit
        val = (val + (1 << (lpc_quant - 1))) >> lpc_quant;
        val += d + prediction_error;
        buf[i] = sign_extend(val, bps);

        // Adapt LPC coefficients
        let mut prediction_error = prediction_error;
        let prediction_error_sign = prediction_error.signum();
        if prediction_error_sign != 0 {
            for j in 0..lpc_order {
                let val = d - buf[pred_index + j];
                let sign = val.signum() * prediction_error_sign;
                lpc_coefs[j] -= sign as i16;
                let val = val * sign;
                prediction_error -= (val >> lpc_quant) * (j as i32 + 1);

                if prediction_error * prediction_error_sign <= 0 {
                    break;
                }
            }
        }
    }
}

fn unmix_stereo(buf: &mut [&mut [i32]; 2], mix_bits: u8, mix_res: i8) {
    debug_assert_eq!(buf[0].len(), buf[1].len());

    let mix_res = mix_res as i32;
    let num_samples = min(buf[0].len(), buf[1].len());

    for i in 0..num_samples {
        let u = buf[0][i];
        let v = buf[1][i];

        let r = u - ((v * mix_res) >> mix_bits);
        let l = r + v;

        buf[0][i] = l;
        buf[1][i] = r;
    }
}

fn append_extra_bits<'a>(reader: &mut BitCursor<'a>,
                         buf: &mut [&mut [i32]; 2],
                         channels: u8,
                         bytes_shifted: u8)
                         -> Result<(), ()> {
    debug_assert_eq!(buf[0].len(), buf[1].len());

    let channels = min(channels as usize, buf.len());
    let num_samples = min(buf[0].len(), buf[1].len());
    let shift = bytes_shifted as usize * 8;

    for i in 0..num_samples {
        for j in 0..channels {
            let extra_bits = try!(reader.read_u16(shift)) as i32;
            buf[j][i] = (buf[j][i] << shift) | extra_bits as i32;
        }
    }

    Ok(())
}
