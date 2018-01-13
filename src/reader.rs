use std::io::{Read, Seek};
use std::marker::PhantomData;

use {Decoder, Sample, StreamInfo};
use mp4::Mp4PacketReader;

pub struct Reader<R> {
    packet_buf: Box<[u8]>,
    packet_reader: Mp4PacketReader<R>,
    decoder: Decoder,
    samples: Box<[i32]>,
    sample_len: usize,
    sample_pos: usize,
}

impl<R: Read + Seek> Reader<R> {
    pub fn new(reader: R) -> Result<Reader<R>, ()> {
        let packet_reader = Mp4PacketReader::new(reader)?;
        let decoder = Decoder::new(packet_reader.stream_info().clone());

        Ok(Reader {
            packet_buf: Box::new([]),
            packet_reader: packet_reader,
            decoder: decoder,
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

    fn decode_next_packet(&mut self) -> Result<Option<()>, ()> {
        // Allocate buffers if required
        if self.packet_buf.is_empty() {
            self.packet_buf = vec![0; self.packet_reader.max_packet_len() as usize].into();
        }
        if self.samples.is_empty() {
            let max_samples = self.decoder.stream_info().max_samples_per_packet() as usize;
            self.samples = vec![0; max_samples].into();
        }

        // Read the next packet
        let packet_buf_len = self.packet_reader
            .next_packet_into(&mut self.packet_buf)
            .map_err(|_| ())?;
        if packet_buf_len == 0 {
            return Ok(None);
        }
        let packet_buf = &self.packet_buf[..packet_buf_len];

        // Decode the next packet
        let samples = self.decoder.decode_packet(packet_buf, &mut self.samples)?;
        self.sample_len = samples.len();
        self.sample_pos = 0;

        Ok(Some(()))
    }

    fn next_sample<S: Sample>(&mut self) -> Option<Result<S, ()>> {
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

pub struct Samples<'a, R: 'a, S: 'a> {
    reader: &'a mut Reader<R>,
    phantom: PhantomData<Box<[S]>>,
}

impl<'a, R: 'a + Read + Seek, S: Sample> Iterator for Samples<'a, R, S> {
    type Item = Result<S, ()>;

    fn next(&mut self) -> Option<Result<S, ()>> {
        self.reader.next_sample()
    }
}

pub struct IntoSamples<R, S> {
    reader: Reader<R>,
    phantom: PhantomData<Box<[S]>>,
}

impl<R: Read + Seek, S: Sample> Iterator for IntoSamples<R, S> {
    type Item = Result<S, ()>;

    fn next(&mut self) -> Option<Result<S, ()>> {
        self.reader.next_sample()
    }
}
