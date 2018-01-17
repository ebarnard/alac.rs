use std::cmp::min;

use {invalid_data, InvalidData, StreamInfo};
use bitcursor::BitCursor;

/// A type that can be used to represent audio samples.
pub trait Sample: Copy + private::Sealed {
    /// Constructs `Self` from a right-aligned sample with bit depth `bits`.
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

mod private {
    /// Sealed prevents other crates from implementing any traits that use it.
    pub trait Sealed {}
    impl Sealed for i16 {}
    impl Sealed for i32 {}
}

/// An ALAC packet decoder.
pub struct Decoder {
    config: StreamInfo,
    buf: Box<[i32]>,
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
    /// Creates a `Decoder` for a stream described by the `StreamInfo`.
    pub fn new(config: StreamInfo) -> Decoder {
        Decoder {
            buf: vec![0; config.frame_length as usize * 2].into_boxed_slice(),
            config,
        }
    }

    /// Returns the `StreamInfo` used to create this decoder.
    pub fn stream_info(&self) -> &StreamInfo {
        &self.config
    }

    /// Decodes an ALAC packet into `out`.
    ///
    /// Channels are interleaved, e.g. for a stereo packet `out` would contains samples in the
    /// order `[left, right, left, right, ..]`.
    ///
    /// Panics if `out` is shorter than `StreamInfo::max_samples_per_packet`.
    pub fn decode_packet<'a, S: Sample>(
        &mut self,
        packet: &[u8],
        out: &'a mut [S],
    ) -> Result<&'a [S], InvalidData> {
        let mut reader = BitCursor::new(packet);

        let mut channel_index = 0;
        let mut frame_samples = None;

        assert!(out.len() >= self.config.max_samples_per_packet() as usize);
        assert!(S::bits() >= self.config.bit_depth);

        loop {
            let tag = reader.read_u8(3)?;

            match tag {
                tag @ ID_SCE | tag @ ID_LFE | tag @ ID_CPE => {
                    let element_channels = match tag {
                        ID_SCE => 1,
                        ID_LFE => 1,
                        ID_CPE => 2,
                        _ => unreachable!(),
                    };

                    // Check that there aren't too many channels in this packet.
                    if channel_index + element_channels > self.config.num_channels {
                        return Err(invalid_data("packet contains more channels than expected"));
                    }

                    let element_samples = decode_audio_element(
                        self,
                        &mut reader,
                        out,
                        channel_index,
                        element_channels,
                    )?;

                    // Check that the number of samples are consistent within elements of a frame.
                    if let Some(frame_samples) = frame_samples {
                        if frame_samples != element_samples {
                            return Err(invalid_data(
                                "all channels in a packet must contain the same number of samples",
                            ));
                        }
                    } else {
                        frame_samples = Some(element_samples);
                    }

                    channel_index += element_channels;
                }
                ID_CCE | ID_PCE => {
                    return Err(invalid_data("packet cce and pce elements are unsupported"));
                }
                ID_DSE => {
                    // data stream element -- parse but ignore

                    // the tag associates this data stream element with a given audio element
                    // Unused
                    let _element_instance_tag = reader.read_u8(4)?;
                    let data_byte_align_flag = reader.read_bit()?;

                    // 8-bit count or (8-bit + 8-bit count) if 8-bit count == 255
                    let mut skip_bytes = reader.read_u8(8)? as usize;
                    if skip_bytes == 255 {
                        skip_bytes += reader.read_u8(8)? as usize;
                    }

                    // the align flag means the bitstream should be byte-aligned before reading the
                    // following data bytes
                    if data_byte_align_flag {
                        reader.skip_to_byte()?;
                    }

                    reader.skip(skip_bytes * 8)?;
                }
                ID_FIL => {
                    // fill element -- parse but ignore

                    // 4-bit count or (4-bit + 8-bit count) if 4-bit count == 15
                    // - plus this weird -1 thing I still don't fully understand
                    let mut skip_bytes = reader.read_u8(4)? as usize;
                    if skip_bytes == 15 {
                        skip_bytes += reader.read_u8(8)? as usize - 1
                    }

                    reader.skip(skip_bytes * 8)?;
                }
                ID_END => {
                    // We've finished decoding the frame. Skip to the end of this byte. There may
                    // be data left in the packet.
                    // TODO: Should we throw an error about leftover data.
                    reader.skip_to_byte()?;

                    // Check that there were as many channels in the packet as there ought to be.
                    if channel_index != self.config.num_channels {
                        return Err(invalid_data("packet contains fewer channels than expected"));
                    }

                    let frame_samples = frame_samples.unwrap_or(self.config.frame_length);
                    return Ok((&out[..frame_samples as usize * channel_index as usize]));
                }
                // `tag` is 3 bits long and we've exhaused all 8 options.
                _ => unreachable!(),
            }
        }
    }
}

fn decode_audio_element<'a, S: Sample>(
    this: &mut Decoder,
    reader: &mut BitCursor<'a>,
    out: &mut [S],
    channel_index: u8,
    element_channels: u8,
) -> Result<u32, InvalidData> {
    // Unused
    let _element_instance_tag = reader.read_u8(4)?;

    let unused = reader.read_u16(12)?;
    if unused != 0 {
        return Err(invalid_data("unused channel header bits must be zero"));
    }

    // read the 1-bit "partial frame" flag, 2-bit "shift-off" flag & 1-bit "escape" flag
    let partial_frame = reader.read_bit()?;

    let sample_shift_bytes = reader.read_u8(2)?;
    if sample_shift_bytes > 2 {
        return Err(invalid_data(
            "channel sample shift must not be greater than 16",
        ));
    }
    let sample_shift = sample_shift_bytes * 8;

    let is_uncompressed = reader.read_bit()?;

    // check for partial frame to override requested numSamples
    let num_samples = if partial_frame {
        // TODO: this could change within a frame. That would be bad
        let num_samples = reader.read_u32(32)?;

        if num_samples > this.config.frame_length {
            return Err(invalid_data("channel contains more samples than expected"));
        }

        num_samples as usize
    } else {
        this.config.frame_length as usize
    };

    if !is_uncompressed {
        let (buf_u, buf_v) = this.buf.split_at_mut(this.config.frame_length as usize);
        let mut mix_buf = [&mut buf_u[..num_samples], &mut buf_v[..num_samples]];

        let chan_bits = this.config.bit_depth - sample_shift + element_channels - 1;
        if chan_bits > 32 {
            // unimplemented - could in theory be 33
            return Err(invalid_data("channel bit depth cannot be greater than 32"));
        }

        // compressed frame, read rest of parameters
        let mix_bits: u8 = reader.read_u8(8)?;
        let mix_res: i8 = reader.read_u8(8)? as i8;

        let mut lpc_mode = [0; 2]; //u8
        let mut lpc_quant = [0; 2]; //u32
        let mut pb_factor = [0; 2]; //u16
        let mut lpc_order = [0; 2]; //u8
        let mut lpc_coefs = [[0; 32]; 2]; //i16*

        for i in 0..(element_channels as usize) {
            lpc_mode[i] = reader.read_u8(4)?;
            lpc_quant[i] = reader.read_u8(4)? as u32;
            pb_factor[i] = reader.read_u8(3)? as u16;
            lpc_order[i] = reader.read_u8(5)?;

            // Coefficients are used in reverse order of storage for prediction
            for j in (0..lpc_order[i] as usize).rev() {
                lpc_coefs[i][j] = reader.read_u16(16)? as i16;
            }
        }

        let extra_bits_reader = if sample_shift != 0 {
            let extra_bits_reader = reader.clone();
            reader.skip((sample_shift as usize) * num_samples * element_channels as usize)?;
            Some(extra_bits_reader)
        } else {
            None
        };

        // TODO: Tidy and comment these steps see below for an example
        // https://github.com/ruud-v-a/claxon/blob/master/src/subframe.rs
        // It should be possible to it without allocating buffers quite easily
        for i in 0..(element_channels as usize) {
            rice_decompress(
                reader,
                &this.config,
                &mut mix_buf[i],
                chan_bits,
                pb_factor[i],
            )?;

            if lpc_mode[i as usize] == 15 {
                // the special "numActive == 31" mode can be done in-place
                lpc_predict_order_31(mix_buf[i], chan_bits);
            } else if lpc_mode[i as usize] > 0 {
                return Err(invalid_data("invalid lpc mode"));
            }

            // We have a seperate function for this
            assert!(lpc_order[i] != 31);

            let lpc_coefs = &mut lpc_coefs[i][..lpc_order[i] as usize];
            lpc_predict(mix_buf[i], chan_bits, lpc_coefs, lpc_quant[i]);
        }

        if element_channels == 2 && mix_res != 0 {
            unmix_stereo(&mut mix_buf, mix_bits, mix_res);
        }

        // now read the shifted values into the shift buffer
        // We directly apply the shifts to avoid needing a buffer
        if let Some(mut extra_bits_reader) = extra_bits_reader {
            append_extra_bits(
                &mut extra_bits_reader,
                &mut mix_buf,
                element_channels,
                sample_shift,
            )?;
        }

        for i in 0..num_samples {
            for j in 0..element_channels as usize {
                let sample = mix_buf[j][i];

                let idx = i * this.config.num_channels as usize + channel_index as usize + j;

                out[idx] = S::from_decoder(sample, this.config.bit_depth);
            }
        }
    } else {
        // uncompressed frame, copy data into the mix buffers to use common output code

        // Here we deviate here from the reference implementation and just copy
        // straight to the output buffer.

        if sample_shift != 0 {
            return Err(invalid_data(
                "sample shift cannot be greater than zero for uncompressed channels",
            ));
        }

        for i in 0..num_samples {
            for j in 0..element_channels as usize {
                let sample = reader.read_u32(this.config.bit_depth as usize)? as i32;

                let idx = i * this.config.num_channels as usize + channel_index as usize + j;

                out[idx] = S::from_decoder(sample, this.config.bit_depth);
            }
        }
    }

    Ok(num_samples as u32)
}

#[inline]
fn decode_rice_symbol<'a>(
    reader: &mut BitCursor<'a>,
    m: u32,
    k: u8,
    bps: u8,
) -> Result<u32, InvalidData> {
    // Rice coding encodes a symbol S as the product of a quotient Q and a
    // modulus M added to a remainder R. Q is encoded in unary (Q 1s followed
    // by a 0) and R in binary in K bits.
    //
    // S = Q × M + R where M = 2^K

    // K cannot be zero as a modulus is 2^K - 1 is used instead of 2^K.
    debug_assert!(k != 0);

    let k = k as usize;

    // First we need to try to read Q which is encoded in unary and is at most
    // 9. If it is greater than 8 the entire symbol is simply encoded in binary
    // after Q.
    let mut q = 0;
    while q != 9 && reader.read_bit()? == true {
        q += 1;
    }

    if q == 9 {
        return Ok(reader.read_u32(bps as usize)?);
    }

    // A modulus of 2^K - 1 is used instead of 2^K. Therefore if K = 1 then
    // M = 1 and there is no remainder (here K cannot be 0 as it comes from
    // log_2 which cannot be 0). This is presumably an optimisation that aims
    // to store small numbers more efficiently.
    if k == 1 {
        return Ok(q);
    }

    // Next we read the remainder which is at most K bits. If it is zero it is
    // stored as K - 1 zeros. Otherwise it is stored in K bits as R + 1. This
    // saves one bit in cases where the remainder is zero.
    let mut r = reader.read_u32(k - 1)?;
    if r > 0 {
        let extra_bit = reader.read_bit()? as u32;
        r = (r << 1) + extra_bit - 1;
    }

    // Due to the issue mentioned in rice_decompress we use a parameter for m
    // rather than calculating it here (e.g. let mut s = (q << k) - q);
    let s = q * m + r;

    Ok(s)
}

fn rice_decompress<'a>(
    reader: &mut BitCursor<'a>,
    config: &StreamInfo,
    buf: &mut [i32],
    bps: u8,
    pb_factor: u16,
) -> Result<(), InvalidData> {
    #[inline(always)]
    fn log_2(x: u32) -> u32 {
        31 - (x | 1).leading_zeros()
    }

    let mut rice_history: u32 = config.mb as u32;
    let rice_history_mult = (config.pb as u32 * pb_factor as u32) / 4;
    let k_max = config.kb;
    let mut sign_modifier = 0;

    let mut i = 0;
    while i < buf.len() {
        let k = log_2((rice_history >> 9) + 3);
        let k = min(k as u8, k_max);
        // See below for info on the m thing
        let m = (1 << k) - 1;
        let val = decode_rice_symbol(reader, m, k, bps)?;
        // The least significant bit of val is the sign bit - the plus is weird tho
        // if val and sgn mod = 0 then nothing happens
        // if one is 1 the lsb = 1
        // val & 1 = 1 => val is all 1s => flip all the bits
        // if they are both 1 then val_eff += 2
        // val & 1 = 0 => nothing happens...?
        let val = val + sign_modifier;
        sign_modifier = 0;
        // As lsb sign bit right shift by 1
        buf[i] = ((val >> 1) as i32) ^ -((val & 1) as i32);

        // Update the history value
        if val > 0xffff {
            rice_history = 0xffff;
        } else {
            // Avoid += as that has a tendency to underflow
            rice_history = (rice_history + val * rice_history_mult) -
                           ((rice_history * rice_history_mult) >> 9);
        }

        // There may be a compressed block of zeros. See if there is.
        if (rice_history < 128) && (i + 1 < buf.len()) {
            // calculate rice param and decode block size
            let k = rice_history.leading_zeros() - 24 + ((rice_history + 16) >> 6);
            // The maximum value k above can take is 7. The rice limit seems to always be higher
            // than this. This is called infrequently enough that the if statement below should
            // have a minimal effect on performance.
            if k as u8 > k_max {
                debug_assert!(
                    false,
                    "k ({}) greater than rice limit ({}). Unsure how to continue.",
                    k,
                    k_max
                );
            }

            // Apple version
            let k = k as u8;
            let wb_local = (1 << k_max) - 1;
            let m = ((1 << k) - 1) & wb_local;
            // FFMPEG version
            // let k = min(k as u8, k_max);
            // let mz = ((1 << k) - 1);
            // End versions

            let zero_block_len = decode_rice_symbol(reader, m, k, 16)? as usize;

            if zero_block_len > 0 {
                if zero_block_len >= buf.len() - i {
                    return Err(invalid_data(
                        "zero block contains too many samples for channel",
                    ));
                }
                // TODO: Use memset equivalent here.
                let buf = &mut buf[i + 1..];
                for j in 0..zero_block_len {
                    buf[j] = 0;
                }
                i += zero_block_len;
            }
            if zero_block_len <= 0xffff {
                sign_modifier = 1;
            }
            rice_history = 0;
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
    // When lpc_order is 31 samples are encoded using differential coding. Samples values are the
    // sum of the previous and the difference between the previous and current sample.
    for i in 1..buf.len() {
        buf[i] = sign_extend(buf[i] + buf[i - 1], bps);
    }
}

fn lpc_predict(buf: &mut [i32], bps: u8, lpc_coefs: &mut [i16], lpc_quant: u32) {
    let lpc_order = lpc_coefs.len();

    // Prediction needs lpc_order + 1 previous decoded samples.
    for i in 1..min(lpc_order + 1, buf.len()) {
        buf[i] = sign_extend(buf[i] + buf[i - 1], bps);
    }

    for i in (lpc_order + 1)..buf.len() {
        // The (lpc_order - 1)'th predicted sample is used as the mean signal value for this
        // prediction.
        let mean = buf[i - lpc_order - 1];

        // The previous lpc_order samples are used to predict this sample.
        let buf = &mut buf[i - lpc_order..i + 1];

        // Predict the next sample using linear predictive coding.
        let mut predicted = 0;
        for (x, coef) in buf.iter().zip(lpc_coefs.iter()) {
            predicted += (x - mean) * (*coef as i32);
        }

        // Round up to and then truncate by lpc_quant bits.
        // 1 << (lpc_quant - 1) sets the (lpc_quant - 1)'th bit.
        let predicted = (predicted + (1 << (lpc_quant - 1))) >> lpc_quant;

        // Store the sample for output and to be used in the next prediction.
        let prediction_error = buf[lpc_order];
        let sample = predicted + mean + prediction_error;
        buf[lpc_order] = sign_extend(sample, bps);

        if prediction_error != 0 {
            // The prediction was not exact so adjust LPC coefficients to try to reduce the size
            // of the next prediction error. Add or subtract 1 from each coefficient until the
            // sign of error has changed or we run out of coefficients to adjust.
            let error_sign = prediction_error.signum();

            // This implementation always uses a positive prediction error.
            let mut prediction_error = error_sign * prediction_error;

            for j in 0..lpc_order {
                let predicted = buf[j] - mean;
                let sign = predicted.signum() * error_sign;
                lpc_coefs[j] += sign as i16;
                // Update the prediction error now we have changed a coefficient.
                prediction_error -= error_sign * (predicted * sign >> lpc_quant) * (j as i32 + 1);
                // Stop updating coefficients if the prediction error changes sign.
                if prediction_error <= 0 {
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

fn append_extra_bits<'a>(
    reader: &mut BitCursor<'a>,
    buf: &mut [&mut [i32]; 2],
    channels: u8,
    sample_shift: u8,
) -> Result<(), InvalidData> {
    debug_assert_eq!(buf[0].len(), buf[1].len());

    let channels = min(channels as usize, buf.len());
    let num_samples = min(buf[0].len(), buf[1].len());
    let sample_shift = sample_shift as usize;

    for i in 0..num_samples {
        for j in 0..channels {
            let extra_bits = reader.read_u16(sample_shift)? as i32;
            buf[j][i] = (buf[j][i] << sample_shift) | extra_bits as i32;
        }
    }

    Ok(())
}
