extern crate alac;

#[test]
fn main() {
	let cookie_bytes = include_bytes!("data/magic_cookie.bin");

	let cookie = alac::DecoderConfig::from_cookie(cookie_bytes).unwrap();

	let comparison = alac::DecoderConfig {
		frame_length: 4096,
		compatible_version: 0,
	 	bit_depth: 16,
		pb: 40,
		mb: 10,
		kb: 14,
		num_channels: 2,
		max_run: 255,
		max_frame_bytes: 0,
		avg_bit_rate: 0,
		sample_rate: 44100
	};

	assert_eq!(cookie, comparison);
}
