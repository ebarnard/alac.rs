use std::cmp;

const U32_BITS: usize = 32;

#[derive(Clone)]
pub struct BitCursor<'a> {
    buf: &'a [u8],
    current: u32,
    current_len: u8,
    current_pos: u8,
}

#[derive(Debug)]
pub struct NotEnoughData;

#[derive(Debug)]
pub struct BufferTooLong;

impl<'a> BitCursor<'a> {
    pub fn new(buf: &'a [u8]) -> Result<BitCursor<'a>, BufferTooLong> {
        if buf.len() > usize::max_value() << 3 {
            return Err(BufferTooLong);
        }

        let mut cursor = BitCursor {
            buf,
            current: 0,
            current_len: 0,
            current_pos: 0,
        };
        cursor.advance();
        Ok(cursor)
    }

    #[inline]
    pub fn read_bit(&mut self) -> Result<bool, NotEnoughData> {
        Ok(match self.read_u32(1)? {
            0 => false,
            1 => true,
            _ => unreachable!(),
        })
    }

    #[inline]
    pub fn read_u8(&mut self, bits: usize) -> Result<u8, NotEnoughData> {
        assert!(bits <= 8);
        debug_assert!(bits > 0);

        Ok(self.read_u32(bits)? as u8)
    }

    #[inline]
    pub fn read_u16(&mut self, bits: usize) -> Result<u16, NotEnoughData> {
        assert!(bits <= 16);
        debug_assert!(bits > 0);

        Ok(self.read_u32(bits)? as u16)
    }

    #[inline]
    pub fn read_u32(&mut self, bits: usize) -> Result<u32, NotEnoughData> {
        assert!(bits <= 32);
        debug_assert!(bits > 0);

        self.check_enough_bits(bits)?;

        debug_assert!(self.current_pos < self.current_len);

        let val = self.current << self.current_pos;
        let bits_remaining = bits.checked_sub((self.current_len - self.current_pos) as usize);
        let val = if let Some(bits_remaining) = bits_remaining {
            // We are reading to or past the end of self.current so we must advance to the next byte.
            let prev_pos = self.current_pos;
            self.advance();
            self.current_pos = bits_remaining as u8;
            // This is a branchless and non-overflowing version of
            // `val | (self.current >> (U32_BITS - prev_pos as usize))`.
            val | ((self.current >> (U32_BITS - 1 - prev_pos as usize)) >> 1)
        } else {
            // We are not reading to or past the end of self.current so only increment the bit position.
            self.current_pos += bits as u8;
            val
        };
        Ok(val >> (U32_BITS - bits))
    }

    #[inline]
    pub fn skip(&mut self, bits: usize) -> Result<(), NotEnoughData> {
        self.check_enough_bits(bits)?;

        if let Some(skip_buf_bits) =
            bits.checked_sub((self.current_len - self.current_pos) as usize)
        {
            // Skip skip_buf_bits bits and refill self.current
            self.buf = &self.buf[skip_buf_bits >> 3..];
            self.advance();
            self.current_pos = (skip_buf_bits & 7) as u8;
        } else {
            // We aren't skipping past the end of self.current
            self.current_pos += bits as u8;
        }
        Ok(())
    }

    #[inline]
    pub fn skip_to_byte(&mut self) -> Result<(), NotEnoughData> {
        let pos_into_byte = self.current_pos & 7;
        if pos_into_byte != 0 {
            self.skip(8 - pos_into_byte as usize)
        } else {
            Ok(())
        }
    }

    #[inline]
    fn check_enough_bits(&self, bits: usize) -> Result<(), NotEnoughData> {
        if bits <= (self.buf.len() << 3) + (self.current_len - self.current_pos) as usize {
            Ok(())
        } else {
            Err(NotEnoughData)
        }
    }

    fn advance(&mut self) {
        let bytes_to_read = cmp::min(4, self.buf.len());
        let (left, right) = self.buf.split_at(bytes_to_read);
        let mut bytes = [0; 4];
        (&mut bytes[0..bytes_to_read]).copy_from_slice(&left);
        self.current = ((bytes[0] as u32) << 24)
            | ((bytes[1] as u32) << 16)
            | ((bytes[2] as u32) << 8)
            | bytes[3] as u32;
        self.buf = right;
        self.current_len = bytes_to_read as u8 * 8;
        self.current_pos = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::BitCursor;

    #[test]
    fn skip_to_byte() {
        let data = &[0xde, 0xad];
        let mut reader = BitCursor::new(data).unwrap();
        reader.read_u8(5).unwrap();
        reader.skip_to_byte().unwrap();
        assert_eq!(reader.read_u8(8).unwrap(), 0xad);
    }
}
