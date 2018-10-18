use std::error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::marker::PhantomData;

use {Decoder, InvalidData, Sample, StreamInfo};

/// The format of an ALAC file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    #[cfg(feature = "caf")]
    Caf,
    #[cfg(feature = "mp4")]
    Mp4,
    #[doc(hidden)]
    __Nonexhaustive,
}

/// An error when reading an ALAC file using a `Reader`.
///
/// A `ReadError::Decoder` will occur if the current packet is invalid. If more samples are read
/// the reader will skip to the next packet.
#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    UnsupportedFormat,
    Format(Format, InvalidData),
    Decoder(InvalidData),
}

impl error::Error for ReadError {
    fn description(&self) -> &str {
        match *self {
            ReadError::Io(_) => "IO error",
            ReadError::UnsupportedFormat => "unsupported format",
            ReadError::Format(_, _) => "format error",
            ReadError::Decoder(_) => "decoder error",
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        match *self {
            ReadError::Io(ref err) => Some(err),
            ReadError::UnsupportedFormat => return None,
            ReadError::Format(_, ref err) => Some(err),
            ReadError::Decoder(ref err) => Some(err),
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ReadError::Io(ref err) => write!(f, "IO error: {}", err),
            ReadError::UnsupportedFormat => write!(f, "unsupported format"),
            ReadError::Format(format, ref err) => write!(f, "{:?} error: {}", format, err),
            ReadError::Decoder(ref err) => write!(f, "decoder error: {}", err),
        }
    }
}

impl From<io::Error> for ReadError {
    fn from(err: io::Error) -> ReadError {
        ReadError::Io(err)
    }
}

/// An ALAC reader and decoder supporting `mp4` and `caf` files (if the respective Cargo features
/// are enabled).
pub struct Reader<R: Read + Seek> {
    packet_buf: Vec<u8>,
    packet_reader: PacketReader<R>,
    decoder: Decoder,
}

impl<R: Read + Seek> Reader<R> {
    /// Attempts to create a `Reader` from a seekable byte stream.
    pub fn new(reader: R) -> Result<Reader<R>, ReadError> {
        let (packet_reader, magic_cookie) = PacketReader::new(reader)?;
        let stream_info = StreamInfo::from_cookie(&magic_cookie).map_err(ReadError::Decoder)?;

        Ok(Reader {
            packet_buf: Vec::new(),
            packet_reader,
            decoder: Decoder::new(stream_info),
        })
    }

    /// Returns the format of this ALAC file.
    pub fn format(&self) -> Format {
        self.packet_reader.format()
    }

    /// Returns a `StreamInfo` describing the ALAC stream in this file.
    pub fn stream_info(&self) -> &StreamInfo {
        self.decoder.stream_info()
    }

    /// Returns an iterator over the samples in the ALAC stream.
    ///
    /// Channels are interleaved, e.g. for a stereo stream they would be yielded in the order
    /// `[left, right, left, right, ..]`.
    pub fn into_samples<S: Sample>(self) -> Samples<R, S> {
        Samples {
            reader: self,
            samples: Vec::new(),
            sample_len: 0,
            sample_pos: 0,
        }
    }

    /// Returns an iterator-like type that decodes packets into a user-provided buffer.
    pub fn into_packets<S: Sample>(self) -> Packets<R, S> {
        Packets {
            reader: self,
            phantom: PhantomData,
        }
    }

    fn decode_next_packet_into<'a, S: Sample>(
        &mut self,
        out: &'a mut [S],
    ) -> Result<Option<&'a [S]>, ReadError> {
        // Read the next packet
        self.packet_reader.next_packet_into(&mut self.packet_buf)?;
        if self.packet_buf.is_empty() {
            return Ok(None);
        }

        // Decode the next packet
        let samples = self
            .decoder
            .decode_packet(&self.packet_buf, out)
            .map_err(|err| ReadError::Decoder(err))?;

        if samples.len() == 0 {
            Ok(None)
        } else {
            Ok(Some(samples))
        }
    }
}

/// An iterator that yields samples of type `S` read from a `Reader`.
pub struct Samples<R: Read + Seek, S> {
    reader: Reader<R>,
    samples: Vec<S>,
    sample_len: usize,
    sample_pos: usize,
}

impl<R: Read + Seek, S: Sample> Samples<R, S> {
    /// Returns the format of this ALAC file.
    pub fn format(&self) -> Format {
        self.reader.format()
    }

    /// Returns a `StreamInfo` describing the ALAC stream in this file.
    pub fn stream_info(&self) -> &StreamInfo {
        self.reader.stream_info()
    }
}

impl<R: Read + Seek, S: Sample> Iterator for Samples<R, S> {
    type Item = Result<S, ReadError>;

    fn next(&mut self) -> Option<Result<S, ReadError>> {
        // Allocate sample buffer if required
        if self.samples.is_empty() {
            let max_samples = self.stream_info().max_samples_per_packet() as usize;
            self.samples = vec![S::from_decoder(0, 16); max_samples];
        }

        // Decode the next packet if we're at the end of the current one.
        if self.sample_pos == self.sample_len {
            self.sample_len = match self.reader.decode_next_packet_into(&mut self.samples) {
                Ok(Some(s)) => s.len(),
                Ok(None) => return None,
                Err(e) => return Some(Err(e)),
            };
            self.sample_pos = 0;
        }

        let sample_pos = self.sample_pos;
        self.sample_pos += 1;
        Some(Ok(self.samples[sample_pos]))
    }
}

/// An iterator-like type that decodes packets into a user-provided buffer.
pub struct Packets<R: Read + Seek, S> {
    reader: Reader<R>,
    phantom: PhantomData<[S]>,
}

impl<R: Read + Seek, S: Sample> Packets<R, S> {
    /// Returns the format of this ALAC file.
    pub fn format(&self) -> Format {
        self.reader.format()
    }

    /// Returns a `StreamInfo` describing the ALAC stream in this file.
    pub fn stream_info(&self) -> &StreamInfo {
        self.reader.stream_info()
    }

    /// Reads the next packet and decodes it into `out`.
    ///
    /// Channels are interleaved, e.g. for a stereo packet `out` would contains samples in the
    /// order `[left, right, left, right, ..]`.
    ///
    /// Panics if `out` is shorter than `StreamInfo::max_samples_per_packet`.
    pub fn next_into<'a>(&mut self, out: &'a mut [S]) -> Result<Option<&'a [S]>, ReadError> {
        self.reader.decode_next_packet_into(out)
    }
}

#[cfg(feature = "caf")]
use caf::CafPacketReader;
#[cfg(feature = "mp4")]
use mp4::Mp4PacketReader;

enum PacketReader<R: Read + Seek> {
    #[cfg(feature = "caf")]
    Caf(CafPacketReader<R>),
    #[cfg(feature = "mp4")]
    Mp4(Mp4PacketReader<R>),
}

impl<R: Read + Seek> PacketReader<R> {
    fn new(mut reader: R) -> Result<(PacketReader<R>, Vec<u8>), ReadError> {
        let mut magic = [0; 8];
        reader.read_exact(&mut magic)?;
        reader.seek(SeekFrom::Current(-(magic.len() as i64)))?;

        match (&magic[0..4], &magic[4..8]) {
            #[cfg(feature = "caf")]
            (b"caff", _) => {
                let (reader, magic_cookie) = CafPacketReader::new(reader)?;
                Ok((PacketReader::Caf(reader), magic_cookie))
            }
            #[cfg(feature = "mp4")]
            (_, b"ftyp") => {
                let (reader, magic_cookie) = Mp4PacketReader::new(reader)?;
                Ok((PacketReader::Mp4(reader), magic_cookie))
            }
            _ => Err(ReadError::UnsupportedFormat),
        }
    }

    fn format(&self) -> Format {
        match *self {
            #[cfg(feature = "caf")]
            PacketReader::Caf(_) => Format::Caf,
            #[cfg(feature = "mp4")]
            PacketReader::Mp4(_) => Format::Mp4,
        }
    }

    fn next_packet_into(&mut self, buf: &mut Vec<u8>) -> Result<(), ReadError> {
        match *self {
            #[cfg(feature = "caf")]
            PacketReader::Caf(ref mut r) => r.next_packet_into(buf),
            #[cfg(feature = "mp4")]
            PacketReader::Mp4(ref mut r) => r.next_packet_into(buf),
        }
    }
}
