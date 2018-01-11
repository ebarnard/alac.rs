// Compares a single 16 bit 2 channel packet against the reference decoder 4th frame of alac.caf
extern crate alac;

use alac::{Decoder, StreamInfo};

#[test]
fn main() {
    let cookie_bytes = include_bytes!("data/magic_cookie.bin");
    let packet = include_bytes!("data/packet_16_bit.bin");

    let mut dec = Decoder::new(StreamInfo::from_cookie(cookie_bytes).unwrap());
    let mut out = vec![0i16; 8192];
    dec.decode_packet(&packet[..8581], &mut out).unwrap();

    let out_comp_bin = include_bytes!("data/out_16_bit.bin");
    let mut out_comp = vec![0i16; 8192];
    for i in 0..out_comp.len() {
        out_comp[i] = ((out_comp_bin[i * 2] as i16)) + ((out_comp_bin[i * 2 + 1] as i16) << 8);
    }

    assert_eq!(out, out_comp);
}
