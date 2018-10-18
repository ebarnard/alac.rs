extern crate caf;

use self::caf::chunks::CafChunk;
use self::caf::{CafError, ChunkType, FormatType};
use std::io::{ErrorKind, Read, Seek};
use std::mem;

use {invalid_data, Format, ReadError};

fn caf_error(msg: &'static str) -> ReadError {
    ReadError::Format(Format::Caf, invalid_data(msg))
}

impl From<CafError> for ReadError {
    fn from(err: CafError) -> ReadError {
        match err {
            CafError::Io(ref err) if err.kind() == ErrorKind::UnexpectedEof => {
                caf_error("unexpected end of stream")
            }
            CafError::Io(err) => ReadError::Io(err),
            CafError::FromUtf8(_) => caf_error("bytes are not valid utf8"),
            CafError::NotCaf => caf_error("not a caf file"),
            CafError::UnsupportedChunkType(_) => caf_error("unsupported chunk type"),
        }
    }
}

pub struct CafPacketReader<R: Read + Seek> {
    reader: caf::CafPacketReader<R>,
}

impl<R: Read + Seek> CafPacketReader<R> {
    pub fn new(reader: R) -> Result<(CafPacketReader<R>, Vec<u8>), ReadError> {
        let mut reader = caf::CafPacketReader::new(reader, vec![ChunkType::MagicCookie])?;
        if reader.audio_desc.format_id != FormatType::AppleLossless {
            return Err(caf_error("does not contain alac data"));
        }
        let magic_cookie = mem::replace(&mut reader.chunks, Vec::new())
            .into_iter()
            .filter_map(|c| match c {
                CafChunk::MagicCookie(d) => Some(d),
                _ => None,
            }).next()
            .ok_or(caf_error("missing magic cookie"))?;
        Ok((CafPacketReader { reader }, magic_cookie))
    }

    pub fn next_packet_into(&mut self, buf: &mut Vec<u8>) -> Result<(), ReadError> {
        if let Some(packet_len) = self.reader.next_packet_size() {
            buf.resize(packet_len, 0);
            self.reader.read_packet_into(&mut buf[..])?;
        } else {
            buf.clear();
        }
        Ok(())
    }
}
