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

        Ok(DecoderConfig {
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
        })
    }

    pub fn from_sdp_format_parameters(params: &str) -> Result<DecoderConfig, ()> {
        use std::str::FromStr;

        fn parse<T: FromStr>(val: Option<&str>) -> Result<T, ()> {
            let val = try!(val.ok_or(()));
            val.parse().map_err(|_| ())
        }

        let mut params = params.split_whitespace();

        let config = DecoderConfig {
            frame_length: try!(parse(params.next())),
            compatible_version: try!(parse(params.next())),
            bit_depth: try!(parse(params.next())),
            pb: try!(parse(params.next())),
            mb: try!(parse(params.next())),
            kb: try!(parse(params.next())),
            num_channels: try!(parse(params.next())),
            max_run: try!(parse(params.next())),
            max_frame_bytes: try!(parse(params.next())),
            avg_bit_rate: try!(parse(params.next())),
            sample_rate: try!(parse(params.next())),
        };

        // Check we haven't been passed too many values
        if params.next().is_some() {
            return Err(());
        }

        Ok(config)
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
            sample_rate: 44100,
        };

        assert_eq!(cookie, comparison);
    }

    #[test]
    fn test_from_sdp_format_parameters() {
        let params = "4096  0   16  40  10  14  2   255 0   0   44100";
        let cookie = DecoderConfig::from_sdp_format_parameters(params).unwrap();

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
            sample_rate: 44100,
        };

        assert_eq!(cookie, comparison);
    }
}
