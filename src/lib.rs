mod bitcursor;
#[cfg(feature = "caf")]
mod caf;
mod dec;
#[cfg(feature = "mp4")]
mod mp4;
#[cfg(any(feature = "caf", feature = "mp4"))]
mod reader;

pub use dec::{Decoder, Sample};
#[cfg(any(feature = "caf", feature = "mp4"))]
pub use reader::{Format, Packets, ReadError, Reader, Samples};

use std::error;
use std::fmt;

/// An error indicating user-provided data is invalid.
///
/// When decoding a packet this error can occur if the packet is invalid or corrupted, or if it has
/// been truncated.
#[derive(Debug)]
pub struct InvalidData {
    message: &'static str,
}

impl error::Error for InvalidData {
    fn description(&self) -> &str {
        self.message
    }
}

impl fmt::Display for InvalidData {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.message)
    }
}

impl From<bitcursor::NotEnoughData> for InvalidData {
    fn from(_: bitcursor::NotEnoughData) -> InvalidData {
        invalid_data("packet is not long enough")
    }
}

impl From<bitcursor::BufferTooLong> for InvalidData {
    fn from(_: bitcursor::BufferTooLong) -> InvalidData {
        invalid_data("packet is too long")
    }
}

fn invalid_data(message: &'static str) -> InvalidData {
    InvalidData { message }
}

/// Codec initialisation parameters for an ALAC stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamInfo {
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

impl StreamInfo {
    /// Creates a `StreamInfo` from a magic cookie. This is often stored in the header of a
    /// container format.
    pub fn from_cookie(mut cookie: &[u8]) -> Result<StreamInfo, InvalidData> {
        // For historical reasons the decoder needs to be resilient to magic cookies vended by older encoders.
        // As specified in the ALACMagicCookieDescription.txt document, there may be additional data encapsulating
        // the ALACSpecificConfig. This would consist of format ('frma') and 'alac' atoms which precede the
        // ALACSpecificConfig.
        // See ALACMagicCookieDescription.txt for additional documentation concerning the 'magic cookie'

        // Make sure we stay in bounds
        if cookie.len() < 24 {
            return Err(invalid_data("magic cookie is not the correct length"));
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
            return Err(invalid_data("magic cookie is not the correct length"));
        }

        StreamInfo {
            frame_length: read_be_u32(&cookie[0..4]),
            compatible_version: cookie[4],
            bit_depth: cookie[5],
            pb: cookie[6],
            mb: cookie[7],
            kb: cookie[8],
            num_channels: cookie[9],
            max_run: read_be_u16(&cookie[10..12]),
            max_frame_bytes: read_be_u32(&cookie[12..16]),
            avg_bit_rate: read_be_u32(&cookie[16..20]),
            sample_rate: read_be_u32(&cookie[20..24]),
        }.validate()
    }

    /// Creates a `StreamInfo` from SDP format specific parameters, i.e. the `fmtp` attribute.
    pub fn from_sdp_format_parameters(params: &str) -> Result<StreamInfo, InvalidData> {
        use std::str::FromStr;

        fn parse<T: FromStr>(val: Option<&str>) -> Result<T, InvalidData> {
            let val = val.ok_or(invalid_data("too few sdp format parameters"))?;
            val.parse()
                .map_err(|_| invalid_data("invalid sdp format parameter"))
        }

        let mut params = params.split_whitespace();

        let info = StreamInfo {
            frame_length: parse(params.next())?,
            compatible_version: parse(params.next())?,
            bit_depth: parse(params.next())?,
            pb: parse(params.next())?,
            mb: parse(params.next())?,
            kb: parse(params.next())?,
            num_channels: parse(params.next())?,
            max_run: parse(params.next())?,
            max_frame_bytes: parse(params.next())?,
            avg_bit_rate: parse(params.next())?,
            sample_rate: parse(params.next())?,
        };

        // Check we haven't been passed too many values
        if params.next().is_some() {
            return Err(invalid_data("too many sdp format parameters"));
        }

        info.validate()
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

    pub fn max_samples_per_packet(&self) -> u32 {
        self.frame_length * self.num_channels as u32
    }

    // TODO: Consider moving this validation to Decoder::new() on next major version bump
    fn validate(self) -> Result<StreamInfo, InvalidData> {
        if self.num_channels == 0 {
            return Err(invalid_data("stream must contain one or more channels"));
        }

        if self
            .frame_length
            .checked_mul(self.num_channels as u32)
            .is_none()
        {
            return Err(invalid_data("overflow calculating max_samples_per_packet"));
        }

        if self.bit_depth == 0 {
            return Err(invalid_data("bit depth must be one or greater"));
        }

        Ok(self)
    }
}

fn read_be_u16(buf: &[u8]) -> u16 {
    assert_eq!(buf.len(), 2);
    ((buf[0] as u16) << 8) | (buf[1] as u16)
}

fn read_be_u32(buf: &[u8]) -> u32 {
    assert_eq!(buf.len(), 4);
    ((buf[0] as u32) << 24) | ((buf[1] as u32) << 16) | ((buf[2] as u32) << 8) | (buf[3] as u32)
}

#[cfg(test)]
mod tests {
    use super::StreamInfo;

    #[test]
    fn test_from_cookie() {
        let cookie_bytes = include_bytes!("../tests/data/magic_cookie.bin");
        let cookie = StreamInfo::from_cookie(cookie_bytes).unwrap();

        let comparison = StreamInfo {
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
            sample_rate: 44100,
        };

        assert_eq!(cookie, comparison);
    }

    #[test]
    fn cookie_must_have_one_or_more_channels() {
        let params = "4096  0   16  40  10  14  0   255 0   0   44100";
        assert!(StreamInfo::from_sdp_format_parameters(params).is_err());
    }

    #[test]
    fn cookie_must_have_nonzero_bit_depth() {
        let params = "4096  0   0  40  10  14  2   255 0   0   44100";
        assert!(StreamInfo::from_sdp_format_parameters(params).is_err());
    }

    #[test]
    fn test_from_sdp_format_parameters() {
        let params = "4096  0   16  40  10  14  2   255 0   0   44100";
        let cookie = StreamInfo::from_sdp_format_parameters(params).unwrap();

        let comparison = StreamInfo {
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
            sample_rate: 44100,
        };

        assert_eq!(cookie, comparison);
    }
}
