use std::error;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};
use std::marker::PhantomData;

use {Decoder, InvalidData, Sample, StreamInfo};

#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Format(&'static str, InvalidData),
    Decoder(InvalidData),
}

impl error::Error for ReadError {
    fn description(&self) -> &str {
        match *self {
            ReadError::Io(_) => "IO error",
            ReadError::Format(_, _) => "format error",
            ReadError::Decoder(_) => "decoder error",
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        Some(match *self {
            ReadError::Io(ref err) => err,
            ReadError::Format(_, ref err) => err,
            ReadError::Decoder(ref err) => err,
        })
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ReadError::Io(ref err) => write!(f, "IO error: {}", err),
            ReadError::Format(format, ref err) => write!(f, "{} error: {}", format, err),
            ReadError::Decoder(ref err) => write!(f, "decoder error: {}", err),
        }
    }
}

impl From<io::Error> for ReadError {
    fn from(err: io::Error) -> ReadError {
        ReadError::Io(err)
    }
}

pub struct Reader<R: Read + Seek> {
    packet_buf: Vec<u8>,
    packet_reader: PacketReader<R>,
    decoder: Decoder,
    samples: Box<[i32]>,
    sample_len: usize,
    sample_pos: usize,
}

impl<R: Read + Seek> Reader<R> {
    pub fn new(reader: R) -> Result<Reader<R>, ReadError> {
        let (packet_reader, magic_cookie) = PacketReader::new(reader)?;
        let stream_info = StreamInfo::from_cookie(&magic_cookie).map_err(ReadError::Decoder)?;

        Ok(Reader {
            packet_buf: Vec::new(),
            packet_reader,
            decoder: Decoder::new(stream_info),
            samples: Box::new([]),
            sample_len: 0,
            sample_pos: 0,
        })
    }

    pub fn stream_info(&self) -> &StreamInfo {
        self.decoder.stream_info()
    }

    pub fn samples<'a, S: 'a + Sample>(&'a mut self) -> Samples<'a, R, S> {
        Samples {
            reader: self,
            phantom: PhantomData,
        }
    }

    pub fn into_samples<S: Sample>(self) -> IntoSamples<R, S> {
        IntoSamples {
            reader: self,
            phantom: PhantomData,
        }
    }

    fn decode_next_packet(&mut self) -> Result<Option<()>, ReadError> {
        // Allocate sample buffer if required
        if self.samples.is_empty() {
            let max_samples = self.decoder.stream_info().max_samples_per_packet() as usize;
            self.samples = vec![0; max_samples].into();
        }

        // Read the next packet
        self.packet_reader.next_packet_into(&mut self.packet_buf)?;
        if self.packet_buf.is_empty() {
            return Ok(None);
        }

        // Decode the next packet
        let samples = self.decoder
            .decode_packet(&self.packet_buf, &mut self.samples)
            .map_err(|err| ReadError::Decoder(err))?;
        self.sample_len = samples.len();
        self.sample_pos = 0;

        Ok(Some(()))
    }

    fn next_sample<S: Sample>(&mut self) -> Option<Result<S, ReadError>> {
        if self.sample_pos == self.sample_len {
            match self.decode_next_packet() {
                Ok(Some(_)) => (),
                Ok(None) => return None,
                Err(e) => return Some(Err(e)),
            }
        }
        let sample_pos = self.sample_pos;
        self.sample_pos += 1;
        let bit_depth = self.decoder.stream_info().bit_depth();
        Some(Ok(S::from_decoder(
            self.samples[sample_pos] >> (32 - bit_depth),
            bit_depth,
        )))
    }
}

pub struct Samples<'a, R: 'a + Read + Seek, S: 'a> {
    reader: &'a mut Reader<R>,
    phantom: PhantomData<Box<[S]>>,
}

impl<'a, R: 'a + Read + Seek, S: Sample> Iterator for Samples<'a, R, S> {
    type Item = Result<S, ReadError>;

    fn next(&mut self) -> Option<Result<S, ReadError>> {
        self.reader.next_sample()
    }
}

pub struct IntoSamples<R: Read + Seek, S> {
    reader: Reader<R>,
    phantom: PhantomData<Box<[S]>>,
}

impl<R: Read + Seek, S: Sample> Iterator for IntoSamples<R, S> {
    type Item = Result<S, ReadError>;

    fn next(&mut self) -> Option<Result<S, ReadError>> {
        self.reader.next_sample()
    }
}

#[cfg(feature = "caf")]
use caf::CafPacketReader;
#[cfg(feature = "mp4")]
use mp4::Mp4PacketReader;

enum PacketReader<R: Read + Seek> {
    #[cfg(feature = "caf")] Caf(CafPacketReader<R>),
    #[cfg(feature = "mp4")] Mp4(Mp4PacketReader<R>),
}

impl<R: Read + Seek> PacketReader<R> {
    fn new(mut reader: R) -> Result<(PacketReader<R>, Vec<u8>), ReadError> {
        let mut magic = [0; 4];
        reader.read_exact(&mut magic)?;
        reader.seek(SeekFrom::Current(-4))?;

        match &magic[..] {
            #[cfg(feature = "caf")]
            b"caff" => {
                let (reader, magic_cookie) = CafPacketReader::new(reader)?;
                Ok((PacketReader::Caf(reader), magic_cookie))
            }
            #[cfg(feature = "mp4")]
            _ => {
                let (reader, magic_cookie) = Mp4PacketReader::new(reader)?;
                Ok((PacketReader::Mp4(reader), magic_cookie))
            }
            #[cfg(not(feature = "mp4"))]
            _ => Err(ReadError::Format("format", "unknown format")),
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
