extern crate mp4parse;

use self::mp4parse::{AudioCodecSpecific, AudioSampleEntry, CodecType, Error, SampleEntry};
use std::io::{ErrorKind, Read, Seek, SeekFrom};

use {invalid_data, Format, ReadError};

fn mp4_error(msg: &'static str) -> ReadError {
    ReadError::Format(Format::Mp4, invalid_data(msg))
}

impl From<Error> for ReadError {
    fn from(err: Error) -> ReadError {
        match err {
            Error::InvalidData(msg) => mp4_error(msg),
            Error::Unsupported(msg) => mp4_error(msg),
            Error::UnexpectedEOF => mp4_error("unexpected end of stream"),
            Error::Io(ref err) if err.kind() == ErrorKind::UnexpectedEof => {
                mp4_error("unexpected end of stream")
            }
            Error::Io(err) => ReadError::Io(err),
            Error::NoMoov => mp4_error("missing moov atom"),
            Error::OutOfMemory => mp4_error("out of memory"),
        }
    }
}

pub struct Mp4PacketReader<R> {
    reader: R,
    chunk_offsets: Vec<u64>,
    sample_sizes: Vec<u32>,
    sample_to_chunk: Vec<SampleToChunk>,
    sample_idx: u32,
}

#[derive(Clone, Copy)]
struct SampleToChunk {
    first_sample: u32,
    first_chunk: u32,
    samples_per_chunk: u32,
}

impl<R: Read + Seek> Mp4PacketReader<R> {
    pub fn new(mut reader: R) -> Result<(Mp4PacketReader<R>, Vec<u8>), ReadError> {
        let mut context = mp4parse::MediaContext::new();
        mp4parse::read_mp4(&mut reader, &mut context)?;

        let track = context
            .tracks
            .into_iter()
            .filter(|track| track.codec_type == CodecType::ALAC)
            .next()
            .ok_or(mp4_error("no alac tracks found"))?;

        let magic_cookie = if let Some(SampleEntry::Audio(AudioSampleEntry {
            codec_specific: AudioCodecSpecific::ALACSpecificBox(alac),
            ..
        })) = track.data
        {
            alac.data
        } else {
            return Err(mp4_error("missing sample entry atom"));
        };

        let chunk_offsets = if let Some(stco) = track.stco {
            stco.offsets
        } else {
            return Err(mp4_error("missing stco (chunk offset) atom"));
        };

        let sample_sizes = if let Some(stsz) = track.stsz {
            stsz.sample_sizes
        } else {
            return Err(mp4_error("missing stsz (sample size) atom"));
        };

        let sample_to_chunk = if let Some(stsc) = track.stsc {
            stsc.samples
        } else {
            return Err(mp4_error("missing stsc (sample to chunk) atom"));
        };

        let sample_to_chunk = sample_to_chunk
            .into_iter()
            .scan((0, 0), |&mut (ref mut samples, ref mut prev_chunk), s| {
                // s.first_chunk is 1 indexed
                let first_chunk = s.first_chunk - 1;
                *samples += (first_chunk - *prev_chunk) * s.samples_per_chunk;
                *prev_chunk = first_chunk;
                Some(SampleToChunk {
                    first_sample: *samples,
                    first_chunk,
                    samples_per_chunk: s.samples_per_chunk,
                })
            }).collect();

        Ok((
            Mp4PacketReader {
                reader,
                chunk_offsets,
                sample_sizes,
                sample_to_chunk,
                sample_idx: 0,
            },
            magic_cookie,
        ))
    }

    pub fn next_packet_into(&mut self, buf: &mut Vec<u8>) -> Result<(), ReadError> {
        let sample_idx = self.sample_idx;
        if sample_idx as usize == self.sample_sizes.len() {
            buf.clear();
            return Ok(());
        }

        // Find the current sample to chunk mapping
        let sample_to_chunk_idx = self.sample_to_chunk
            .binary_search_by_key(&self.sample_idx, |s| s.first_sample)
            // If we are past s.first_sample we want the index of s
            .unwrap_or_else(|i| i - 1);
        let sample_to_chunk = self.sample_to_chunk[sample_to_chunk_idx];
        let samples_per_chunk = sample_to_chunk.samples_per_chunk;

        let chunks_past_first_chunk =
            (sample_idx - sample_to_chunk.first_sample) / samples_per_chunk;
        let samples_into_chunk = sample_idx - chunks_past_first_chunk * samples_per_chunk;

        // Seek to next chunk offset if starting a new chunk
        if samples_into_chunk == 0 {
            let chunk_idx = sample_to_chunk.first_chunk + chunks_past_first_chunk;
            let chunk_offset = self.chunk_offsets[chunk_idx as usize];
            self.reader.seek(SeekFrom::Start(chunk_offset))?;
        }

        let packet_len = self.sample_sizes[sample_idx as usize] as usize;
        buf.resize(packet_len, 0);
        self.reader.read_exact(&mut buf[..])?;
        self.sample_idx += 1;
        Ok(())
    }
}
