use std::io::{ErrorKind, Read, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use tokio::io::Error;

pub trait ReadPascalExt: Read {
    fn read_pascal_string(&mut self) -> Result<String, Error> {
        let mut data = vec![0; self.read_u8()? as usize];

        self.read_exact(&mut data)?;
        match String::from_utf8(data) {
            Ok(r) => Ok(r),
            Err(e) => Err(Error::new(ErrorKind::InvalidData, e)),
        }
    }

    fn read_pascal_buffer(&mut self, max_size: u16) -> Result<Vec<u8>, Error> {
        let size = self.read_u16::<LittleEndian>()?;
        if size > max_size {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("sent data size {size} exceeds maximum {max_size}"),
            ));
        }

        let mut data = vec![0; size as usize];
        self.read_exact(&mut data)?;
        Ok(data)
    }
}

pub trait WritePascalExt: Write {
    fn write_pascal_string(&mut self, string: &str) -> Result<(), Error> {
        let str_data = string.as_bytes();
        if str_data.len() > u8::MAX as usize {
            return Err(Error::new(ErrorKind::InvalidData, "string too long"));
        }
        #[allow(clippy::cast_possible_truncation)]
        self.write_u8(str_data.len() as u8)?;
        self.write_all(str_data)?;
        Ok(())
    }

    fn write_pascal_buffer(&mut self, data: &[u8]) -> Result<(), Error> {
        if data.len() > u16::MAX as usize {
            return Err(Error::new(ErrorKind::InvalidData, "data too long"));
        }

        #[allow(clippy::cast_possible_truncation)]
        self.write_u16::<LittleEndian>(data.len() as u16)?;
        self.write_all(data)?;
        Ok(())
    }
}

impl<R: Read + ?Sized> ReadPascalExt for R {}

impl<W: Write + ?Sized> WritePascalExt for W {}

#[cfg(test)]
mod tests {
    use bytes::{Buf, BufMut, BytesMut};

    use crate::io_util::{ReadPascalExt, WritePascalExt};

    #[test]
    fn test_write_pascal_string() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pascal_string("hello").unwrap();
            buf = w.into_inner();
        }

        assert_eq!(buf.len(), 6);
        assert_eq!(buf[0], 5);
        assert_eq!(char::from(buf[1]), 'h');
        assert_eq!(char::from(buf[2]), 'e');
        assert_eq!(char::from(buf[3]), 'l');
        assert_eq!(char::from(buf[4]), 'l');
        assert_eq!(char::from(buf[5]), 'o');
    }

    #[test]
    fn test_write_pascal_string_too_long() {
        let buf = BytesMut::new();
        let mut w = buf.writer();
        assert!(w.write_pascal_string(&"x".repeat(256)).is_err());
    }

    #[test]
    fn test_read_pascal_string() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pascal_string("hello").unwrap();
            buf = w.into_inner();
        }

        let mut r = buf.reader();
        assert_eq!(r.read_pascal_string().unwrap(), "hello");
    }

    #[test]
    fn test_read_pascal_string_too_short() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pascal_string("hello").unwrap();
            buf = w.into_inner();
        }

        buf = buf.split_off(1);

        let mut r = buf.reader();
        assert!(r.read_pascal_string().is_err());
    }

    #[test]
    fn test_write_pascal_buffer() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pascal_buffer(&[1, 2, 3, 4]).unwrap();
            buf = w.into_inner();
        }

        assert_eq!(buf.len(), 6);
        assert_eq!(buf[0], 4);
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], 1);
        assert_eq!(buf[3], 2);
        assert_eq!(buf[4], 3);
        assert_eq!(buf[5], 4);
    }

    #[test]
    fn test_write_pascal_buffer_too_long() {
        let buf = BytesMut::new();
        let mut w = buf.writer();
        assert!(w
            .write_pascal_buffer(&[1u8].repeat(u16::MAX as usize + 1))
            .is_err());
    }

    #[test]
    fn test_read_pascal_buffer() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pascal_buffer(&[1, 2, 3, 4]).unwrap();
            buf = w.into_inner();
        }

        let mut r = buf.reader();
        assert_eq!(r.read_pascal_buffer(5000).unwrap(), [1, 2, 3, 4]);
    }

    #[test]
    fn test_read_pascal_buffer_too_short() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pascal_buffer(&[1, 2, 3, 4]).unwrap();
            buf = w.into_inner();
        }

        buf = buf.split_off(1);

        let mut r = buf.reader();
        assert!(r.read_pascal_buffer(500).is_err());
    }
}
