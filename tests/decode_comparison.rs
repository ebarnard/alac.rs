#![cfg(any(feature = "caf", feature = "mp4"))]

extern crate alac;
extern crate hound;

use std::fmt;
use std::fs::File;

static ROOT: &'static str = "tests/data/decode_comparison";

static COMPARE_MP4_I16: &'static [(&'static str, &'static str)] =
    &[("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.m4a")];

static COMPARE_MP4_I32: &'static [(&'static str, &'static str)] = &[
    ("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.m4a"),
    ("synth_44100_24_bit.wav", "synth_44100_24_bit_afconvert.m4a"),
];

static COMPARE_CAF_I16: &'static [(&'static str, &'static str)] =
    &[("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.caf")];

static COMPARE_CAF_I32: &'static [(&'static str, &'static str)] = &[
    ("synth_44100_16_bit.wav", "synth_44100_16_bit_afconvert.caf"),
    ("synth_44100_24_bit.wav", "synth_44100_24_bit_afconvert.caf"),
];

#[test]
#[cfg(feature = "mp4")]
fn mp4() {
    for &(wav, alac) in COMPARE_MP4_I16 {
        compare::<i16>(ROOT, wav, alac);
    }
    for &(wav, alac) in COMPARE_MP4_I32 {
        compare::<i32>(ROOT, wav, alac);
    }
}

#[test]
#[cfg(feature = "caf")]
fn caf() {
    for &(wav, alac) in COMPARE_CAF_I16 {
        compare::<i16>(ROOT, wav, alac);
    }
    for &(wav, alac) in COMPARE_CAF_I32 {
        compare::<i32>(ROOT, wav, alac);
    }
}

fn compare<S: Sample>(root: &str, wav: &str, alac: &str) {
    println!("comparing {} to {}", wav, alac);

    let wav = format!("{}/{}", root, wav);
    let alac = format!("{}/{}", root, alac);

    let alac = File::open(alac).expect("failed to open alac file");
    let alac = alac::Reader::new(alac).expect("failed to open alac reader");
    let bit_depth = alac.stream_info().bit_depth();
    let mut alac = alac.into_samples();

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

trait Sample: alac::Sample + hound::Sample + Clone + Copy + fmt::Display + PartialEq {
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
