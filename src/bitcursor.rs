#[derive(Clone)]
pub struct BitCursor<'a> {
    buf: &'a [u8],
    position: usize,
    bit_position: u8,
}

impl<'a> BitCursor<'a> {
    pub fn new(buf: &'a [u8]) -> BitCursor<'a> {
        BitCursor {
            buf: buf,
            position: 0,
            bit_position: 0,
        }
    }

    #[inline]
    pub fn read_bit(&mut self) -> Result<bool, ()> {
        self.read_u8(1).map(|b| {
            match b {
                0 => false,
                1 => true,
                _ => unreachable!(),
            }
        })
    }

    #[inline]
    pub fn read_u8(&mut self, len: usize) -> Result<u8, ()> {
        assert!(len <= 8);
        try!(self.check_avail(len));

        let ret: u16 = ((self.buf[self.position] as u16) << 8) +
                       *self.buf.get(self.position + 1).unwrap_or(&0) as u16;
        let ret = ret << self.bit_position;
        let ret = ret >> (16 - len);

        self.skip_unckecked(len);

        Ok(ret as u8)
    }

    #[inline]
    pub fn peek_u16(&mut self, len: usize) -> Result<u16, ()> {
        assert!(len <= 16);
        try!(self.check_avail(len));

        let ret: u32 = ((self.buf[self.position] as u32) << 16) +
                       ((*self.buf.get(self.position + 1).unwrap_or(&0) as u32) << 8) +
                       *self.buf.get(self.position + 2).unwrap_or(&0) as u32;

        let ret = ret << 8 + self.bit_position;
        let ret = ret >> (32 - len);

        Ok(ret as u16)
    }

    #[inline]
    pub fn read_u16(&mut self, len: usize) -> Result<u16, ()> {
        let ret = try!(self.peek_u16(len));
        self.skip_unckecked(len);
        Ok(ret)
    }

    #[inline]
    pub fn peek_u32(&mut self, len: usize) -> Result<u32, ()> {
        assert!(len <= 32);
        try!(self.check_avail(len));

        let ret: u64 = ((self.buf[self.position] as u64) << 32) +
                       ((*self.buf.get(self.position + 1).unwrap_or(&0) as u64) << 24) +
                       ((*self.buf.get(self.position + 2).unwrap_or(&0) as u64) << 16) +
                       ((*self.buf.get(self.position + 3).unwrap_or(&0) as u64) << 8) +
                       (*self.buf.get(self.position + 4).unwrap_or(&0) as u64);

        let ret = ret << 24 + self.bit_position;
        let ret = ret >> (64 - len);

        Ok(ret as u32)
    }

    #[inline]
    pub fn read_u32(&mut self, len: usize) -> Result<u32, ()> {
        let ret = try!(self.peek_u32(len));
        self.skip_unckecked(len);
        Ok(ret)
    }

    pub fn position(&self) -> (usize, u8) {
        (self.position, self.bit_position)
    }

    #[inline]
    fn check_avail(&self, bit_len: usize) -> Result<(), ()> {
        let pos = self.position + (bit_len + self.bit_position as usize - 1) / 8;
        if pos < self.buf.len() {
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn skip(&mut self, bit_len: usize) -> Result<(), ()> {
        try!(self.check_avail(bit_len));
        self.skip_unckecked(bit_len);
        Ok(())
    }

    #[inline]
    fn skip_unckecked(&mut self, bit_len: usize) {
        let bit_pos = self.bit_position as usize + bit_len;

        self.position += bit_pos >> 3;
        self.bit_position = (bit_pos & 7) as u8;
    }

    pub fn skip_to_byte(&mut self) -> Result<(), ()> {
        let bit_position = self.bit_position;
        if bit_position == 0 {
            Ok(())
        } else {
            self.skip(8 - bit_position as usize)
        }
    }

    pub fn buf(&self) -> &'a [u8] {
        self.buf
    }
}
