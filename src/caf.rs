extern crate caf;

use self::caf::{ChunkType, FormatType};
use self::caf::chunks::CafChunk;
use std::io::{Read, Seek};

use StreamInfo;

pub struct CafPacketReader<R: Read + Seek> {
    reader: caf::CafPacketReader<R>,
    stream_info: StreamInfo,
}

impl<R: Read + Seek> CafPacketReader<R> {
    pub fn new(reader: R) -> Result<CafPacketReader<R>, ()> {
        let reader =
            caf::CafPacketReader::new(reader, vec![ChunkType::MagicCookie]).map_err(|_| ())?;
        if reader.audio_desc.format_id != FormatType::AppleLossless {
            return Err(());
        }
        let stream_info = {
            let cookie = reader
                .chunks
                .iter()
                .filter_map(|c| match c {
                    &CafChunk::MagicCookie(ref d) => Some(d),
                    _ => None,
                })
                .next()
                .ok_or(())?;
            StreamInfo::from_cookie(&cookie)?
        };
        Ok(CafPacketReader {
            reader,
            stream_info,
        })
    }

    pub fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    pub fn next_packet_into(&mut self, buf: &mut Vec<u8>) -> Result<(), ()> {
        if let Some(packet_len) = self.reader.next_packet_size() {
            buf.resize(packet_len, 0);
            self.reader.read_packet_into(&mut buf[..]).map_err(|_| ())?;
        } else {
            buf.clear();
        }
        Ok(())
    }
}
