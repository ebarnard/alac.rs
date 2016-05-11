extern crate byteorder;

mod bitcursor;
mod dec;

pub use dec::{Decoder, Sample};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecoderConfig {
    frame_length: u32,
    compatible_version: u8,
    bit_depth: u8,
    pb: u8, // rice_history_mult
    mb: u8, // rice_initial_history
    kb: u8, // rice_limit
    num_channels: u8,
    max_run: u16,
    max_frame_bytes: u32,
    avg_bit_rate: u32,
    sample_rate: u32,
}

impl DecoderConfig {
    pub fn from_cookie(mut cookie: &[u8]) -> Result<DecoderConfig, ()> {
        use byteorder::{BigEndian, ReadBytesExt};
        use std::io::Cursor;

        // For historical reasons the decoder needs to be resilient to magic cookies vended by older encoders.
        // As specified in the ALACMagicCookieDescription.txt document, there may be additional data encapsulating
        // the ALACSpecificConfig. This would consist of format ('frma') and 'alac' atoms which precede the
        // ALACSpecificConfig.
        // See ALACMagicCookieDescription.txt for additional documentation concerning the 'magic cookie'

        // Make sure we stay in bounds
        if cookie.len() < 24 {
            return Err(());
        };

        // skip format ('frma') atom if present
        if &cookie[4..8] == b"frma" {
            cookie = &cookie[12..];
        }

        // skip 'alac' atom header if present
        if &cookie[4..8] == b"alac" {
            cookie = &cookie[12..];
        }

        // Make sure cookie is long enough
        if cookie.len() < 24 {
            return Err(());
        }

        let mut reader = Cursor::new(cookie);

        // These reads are guarenteed to succeed
        Ok(DecoderConfig {
            frame_length: reader.read_u32::<BigEndian>().unwrap(),
            compatible_version: reader.read_u8().unwrap(),
            bit_depth: reader.read_u8().unwrap(),
            pb: reader.read_u8().unwrap(),
            mb: reader.read_u8().unwrap(),
            kb: reader.read_u8().unwrap(),
            num_channels: reader.read_u8().unwrap(),
            max_run: reader.read_u16::<BigEndian>().unwrap(),
            max_frame_bytes: reader.read_u32::<BigEndian>().unwrap(),
            avg_bit_rate: reader.read_u32::<BigEndian>().unwrap(),
            sample_rate: reader.read_u32::<BigEndian>().unwrap(),
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn bit_depth(&self) -> u8 {
        self.bit_depth
    }

    pub fn channels(&self) -> u8 {
        self.num_channels
    }

    pub fn max_frames_per_packet(&self) -> u32 {
        self.frame_length
    }
}

#[cfg(test)]
mod tests {
    use super::DecoderConfig;

    #[test]
    fn test_from_cookie() {
        let cookie_bytes = include_bytes!("../tests/data/magic_cookie.bin");
        let cookie = DecoderConfig::from_cookie(cookie_bytes).unwrap();

        let comparison = DecoderConfig {
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
}
