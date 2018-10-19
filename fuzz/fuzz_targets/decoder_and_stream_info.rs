#![no_main]
#[macro_use]
extern crate libfuzzer_sys;
extern crate alac;

use alac::*;

fuzz_target!(|data: &[u8]| {
    if data.len() < 24 {
        return;
    }
    let (cookie, packet) = data.split_at(24);

    let stream_info = StreamInfo::from_cookie(cookie);
    let stream_info = if let Ok(s) = stream_info {
        s
    } else {
        return;
    };

    if stream_info.max_samples_per_packet() > 1024 * 50 || stream_info.bit_depth() > 32 {
        return;
    }

    let mut decoder = Decoder::new(stream_info);
    let mut out = [0i32; 1024 * 50];
    let _ = decoder.decode_packet(packet, &mut out);
});
