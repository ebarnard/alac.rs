extern crate byteorder;

mod bitcursor;
mod dec;

pub use dec::{Decoder, Sample};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AlacConfig {
    pub frame_length: u32,
    pub compatible_version: u8,
    pub bit_depth: u8,
    pub pb: u8, // rice_history_mult
    pub mb: u8, // rice_initial_history
    pub kb: u8, // rice_limit
    pub num_channels: u8,
    pub max_run: u16,
    pub max_frame_bytes: u32,
    pub avg_bit_rate: u32,
    pub sample_rate: u32,
}

impl AlacConfig {
    pub fn from_cookie(mut cookie: &[u8]) -> Result<AlacConfig, ()> {
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
        if &cookie[0..4] == b"frma" {
            cookie = &cookie[12..];
        }

        // skip 'alac' atom header if present
        if &cookie[0..4] == b"alac" {
            cookie = &cookie[12..];
        }

        // Make sure cookie is long enough
        if cookie.len() < 24 {
            return Err(());
        }

        let mut reader = Cursor::new(cookie);

        // These reads are guarenteed to succeed
        Ok(AlacConfig {
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
}
