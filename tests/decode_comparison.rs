extern crate alac;
extern crate caf;
extern crate hound;

use caf::{CafPacketReader, ChunkType, FormatType};
use caf::chunks::CafChunk;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek};

static ROOT: &'static str = "tests/data/decode_comparison";

static COMPARE_I16: &'static [(&'static str, &'static str)] = &[
    ("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.caf"),
];

static COMPARE_I32: &'static [(&'static str, &'static str)] = &[
    ("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.caf"),
    ("synth_44100_24_bit.wav", "synth_44100_24_bit_afconvert.caf"),
];

#[test]
fn main() {
    for &(wav, alac) in COMPARE_I16 {
        compare::<i16>(ROOT, wav, alac);
    }
    for &(wav, alac) in COMPARE_I32 {
        compare::<i32>(ROOT, wav, alac);
    }
}

fn compare<S: Sample>(root: &str, wav: &str, alac: &str) {
    println!("comparing {} to {}", wav, alac);

    let wav = format!("{}/{}", root, wav);
    let alac = format!("{}/{}", root, alac);

    let mut wav = hound::WavReader::open(wav)
        .expect("failed to open wav")
        .into_samples::<S>();
    let mut alac =
        AlacReader::new(File::open(alac).expect("failed to open caf")).expect("invalid caf");
    let bit_depth = alac.decoder.stream_info().bit_depth();

    for i in 0.. {
        let wav_sample = wav.next().map(|r| r.map(|s| s.hound_left_align(bit_depth)));
        match (wav_sample, alac.next()) {
            (Some(Ok(ref w)), Some(Ok(ref a))) if w == a => (),
            (Some(Ok(w)), Some(Ok(a))) => {
                panic!("sample {} does not match. wav: {}, alac: {}", i, w, a)
            }
            (None, None) => break,
            (Some(_), None) => panic!("wav longer than alac"),
            (None, Some(_)) => panic!("alac longer than wav"),
            (Some(Err(_)), _) => panic!("wav read error at {}", i),
            (_, Some(Err(AlacReaderError::Alac))) => panic!("alac read error at {}", i),
            (_, Some(Err(AlacReaderError::Caf))) => panic!("caf read error at {}", i),
        }
    }
}

trait Sample
    : alac::Sample + hound::Sample + Clone + Copy + fmt::Display + PartialEq {
    fn zero() -> Self;
    /// Hound samples are right aligned and need to be shifted to compare with alac samples if the
    /// stream bit depth is lower than the sample type bit depth.
    fn hound_left_align(self, bit_depth: u8) -> Self;
}

impl Sample for i16 {
    fn zero() -> Self {
        0
    }

    fn hound_left_align(self, bit_depth: u8) -> Self {
        self << (16 - bit_depth)
    }
}

impl Sample for i32 {
    fn zero() -> Self {
        0
    }

    fn hound_left_align(self, bit_depth: u8) -> Self {
        self << (32 - bit_depth)
    }
}

enum AlacReaderError {
    Alac,
    Caf,
}

struct AlacReader<T, S>
where
    T: Read + Seek,
{
    caf: CafPacketReader<T>,
    decoder: alac::Decoder,
    output: Vec<S>,
    pos: usize,
    len: usize,
}

impl<T, S: Sample> AlacReader<T, S>
where
    T: Read + Seek,
{
    fn new(rdr: T) -> Result<Self, ()> {
        let caf = CafPacketReader::new(rdr, vec![ChunkType::MagicCookie]).map_err(|_| ())?;
        if caf.audio_desc.format_id != FormatType::AppleLossless {
            return Err(());
        }
        let info = {
            let cookie = caf.chunks
                .iter()
                .filter_map(|c| match c {
                    &CafChunk::MagicCookie(ref d) => Some(d),
                    _ => None,
                })
                .next()
                .ok_or(())?;
            alac::StreamInfo::from_cookie(&cookie)?
        };
        Ok(AlacReader {
            caf: caf,
            output: vec![S::zero(); info.max_samples_per_packet() as usize],
            decoder: alac::Decoder::new(info),
            pos: 0,
            len: 0,
        })
    }
}

impl<T, S: Sample> Iterator for AlacReader<T, S>
where
    T: Read + Seek,
{
    type Item = Result<S, AlacReaderError>;

    fn next(&mut self) -> Option<Result<S, AlacReaderError>> {
        if self.pos == self.len {
            let packet = match self.caf.next_packet() {
                Ok(Some(pck)) => pck,
                Ok(None) => return None,
                Err(_) => return Some(Err(AlacReaderError::Caf)),
            };
            let out = match self.decoder.decode_packet(&packet, &mut self.output) {
                Ok(out) => out,
                Err(_) => return Some(Err(AlacReaderError::Alac)),
            };
            self.len = out.len();
            self.pos = 0;
        }
        let sample = self.output[self.pos];
        self.pos += 1;
        Some(Ok(sample))
    }
}
