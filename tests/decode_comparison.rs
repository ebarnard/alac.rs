extern crate alac;
extern crate caf;
extern crate hound;

use alac::StreamInfo;
use caf::{CafPacketReader, ChunkType, FormatType};
use caf::chunks::CafChunk;
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek};

static ROOT: &'static str = "tests/data/decode_comparison";

static COMPARE_MP4_I16: &'static [(&'static str, &'static str)] = &[
    ("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.m4a"),
];

static COMPARE_MP4_I32: &'static [(&'static str, &'static str)] = &[
    ("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.m4a"),
    ("synth_44100_24_bit.wav", "synth_44100_24_bit_afconvert.m4a"),
];

static COMPARE_CAF_I16: &'static [(&'static str, &'static str)] = &[
    ("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.caf"),
];

static COMPARE_CAF_I32: &'static [(&'static str, &'static str)] = &[
    ("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.caf"),
    ("synth_44100_24_bit.wav", "synth_44100_24_bit_afconvert.caf"),
];

#[test]
#[cfg(feature = "mp4")]
fn mp4() {
    for &(wav, alac) in COMPARE_MP4_I16 {
        compare::<i16, _, _>(ROOT, wav, alac, mp4_open);
    }
    for &(wav, alac) in COMPARE_MP4_I32 {
        compare::<i32, _, _>(ROOT, wav, alac, mp4_open);
    }
}

#[cfg(feature = "mp4")]
fn mp4_open<S: Sample>(f: File) -> Result<(alac::IntoSamples<File, S>, StreamInfo), ()> {
    let r = alac::Reader::new(f)?;
    let s = r.stream_info().clone();
    Ok((r.into_samples(), s))
}

#[test]
fn caf() {
    for &(wav, alac) in COMPARE_CAF_I16 {
        compare::<i16, _, _>(ROOT, wav, alac, caf_open);
    }
    for &(wav, alac) in COMPARE_CAF_I32 {
        compare::<i32, _, _>(ROOT, wav, alac, caf_open);
    }
}

fn caf_open<S: Sample>(f: File) -> Result<(AlacReader<File, S>, StreamInfo), ()> {
    let r = AlacReader::new(f)?;
    let s = r.decoder.stream_info().clone();
    Ok((r, s))
}

fn compare<S: Sample, E, I: Iterator<Item = Result<S, E>>>(
    root: &str,
    wav: &str,
    alac: &str,
    alac_open: fn(File) -> Result<(I, StreamInfo), ()>,
) {
    println!("comparing {} to {}", wav, alac);

    let wav = format!("{}/{}", root, wav);
    let alac = format!("{}/{}", root, alac);

    let (mut alac, stream_info) = File::open(alac)
        .map_err(|_| ())
        .and_then(alac_open)
        .expect("failed to open alac");

    let bit_depth = stream_info.bit_depth();
    let mut wav = hound::WavReader::open(wav)
        .expect("failed to open wav")
        .into_samples::<S>()
        .map(|r| r.map(|s| s.hound_left_align(bit_depth)));

    for i in 0.. {
        match (wav.next(), alac.next()) {
            (Some(Ok(ref w)), Some(Ok(ref a))) if w == a => (),
            (Some(Ok(w)), Some(Ok(a))) => {
                panic!("sample {} does not match. wav: {}, alac: {}", i, w, a)
            }
            (None, None) => break,
            (Some(_), None) => panic!("wav longer than alac"),
            (None, Some(_)) => panic!("alac longer than wav"),
            (Some(Err(_)), _) => panic!("wav read error at {}", i),
            (_, Some(Err(_))) => panic!("alac read error at {}", i),
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
