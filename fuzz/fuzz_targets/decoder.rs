#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate alac;

use alac::*;

fuzz_target!(|data: &[u8]| {
    let stream_info =
        StreamInfo::from_cookie(include_bytes!("../../tests/data/magic_cookie.bin")).unwrap();
    let mut decoder = Decoder::new(stream_info);
    let mut out = [0; 1024 * 10];
    let _ = decoder.decode_packet(data, &mut out);
});
