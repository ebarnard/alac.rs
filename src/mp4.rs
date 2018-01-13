use mp4parse::{self, AudioCodecSpecific, AudioSampleEntry, CodecType, SampleEntry};
use std::io::{self, Read, Seek, SeekFrom};

use StreamInfo;

pub struct Mp4PacketReader<R> {
    reader: R,
    stream_info: StreamInfo,
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
    pub fn new(mut reader: R) -> Result<Mp4PacketReader<R>, ()> {
        let mut context = mp4parse::MediaContext::new();
        mp4parse::read_mp4(&mut reader, &mut context).map_err(|_| ())?;

        let track = context
            .tracks
            .into_iter()
            .filter(|track| track.codec_type == CodecType::ALAC)
            .next()
            .ok_or(())?;

        let stream_info = {
            let cookie = if let Some(SampleEntry::Audio(AudioSampleEntry {
                codec_specific: AudioCodecSpecific::ALACSpecificBox(ref alac),
                ..
            })) = track.data
            {
                &alac.data
            } else {
                // missing sample entry atom
                return Err(());
            };
            StreamInfo::from_cookie(cookie).map_err(|_| ())?
        };

        let chunk_offsets = if let Some(stco) = track.stco {
            stco.offsets
        } else {
            // missing stco (chunk offset) atom
            return Err(());
        };

        let sample_sizes = if let Some(stsz) = track.stsz {
            stsz.sample_sizes
        } else {
            // missing stsz (sample size) atom
            return Err(());
        };

        let sample_to_chunk = if let Some(stsc) = track.stsc {
            stsc.samples
        } else {
            // missing stsc (sample to chunk) atom
            return Err(());
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
                    first_chunk: first_chunk,
                    samples_per_chunk: s.samples_per_chunk,
                })
            })
            .collect();

        Ok(Mp4PacketReader {
            reader: reader,
            stream_info: stream_info,
            chunk_offsets: chunk_offsets,
            sample_sizes: sample_sizes,
            sample_to_chunk: sample_to_chunk,
            sample_idx: 0,
        })
    }

    pub fn stream_info(&self) -> &StreamInfo {
        &self.stream_info
    }

    pub fn max_packet_len(&self) -> u32 {
        *self.sample_sizes.iter().max().unwrap_or(&0)
    }

    pub fn next_packet_into(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let sample_idx = self.sample_idx;
        if sample_idx as usize == self.sample_sizes.len() {
            return Ok(0);
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
            self.reader
                .seek(SeekFrom::Start(self.chunk_offsets[chunk_idx as usize]))?;
        }

        let read_len = self.sample_sizes[sample_idx as usize] as usize;
        self.reader.read_exact(&mut buf[..read_len])?;
        self.sample_idx += 1;
        Ok(read_len)
    }
}
