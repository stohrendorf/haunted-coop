use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{ErrorKind, Read, Write};
use tokio::io::Error;

pub trait ReadPascalExt: Read {
    fn read_pstring(&mut self) -> Result<String, Error> {
        let mut data = Vec::new();

        data.resize(self.read_u8()? as usize, 0);
        self.read_exact(&mut data)?;
        match String::from_utf8(data) {
            Ok(r) => Ok(r),
            Err(e) => Err(Error::new(ErrorKind::InvalidData, e)),
        }
    }

    fn read_pbuffer(&mut self, max_size: u16) -> Result<Vec<u8>, Error> {
        let size = self.read_u16::<LittleEndian>()?;
        if size > max_size {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("sent data size {} exceeds maximum {}", size, max_size),
            ));
        }

        let mut data = Vec::new();
        data.resize(size as usize, 0);
        self.read_exact(&mut data)?;
        Ok(data)
    }
}

pub trait WritePascalExt: Write {
    fn write_pstring(&mut self, string: &str) -> Result<(), Error> {
        let str_data = string.as_bytes();
        if str_data.len() > u8::MAX as usize {
            return Err(Error::new(ErrorKind::InvalidData, "string too long"));
        }
        self.write_u8(str_data.len() as u8)?;
        self.write_all(str_data)?;
        Ok(())
    }

    fn write_pbuffer(&mut self, data: &[u8]) -> Result<(), Error> {
        if data.len() > u16::MAX as usize {
            return Err(Error::new(ErrorKind::InvalidData, "data too long"));
        }

        self.write_u16::<LittleEndian>(data.len() as u16)?;
        self.write_all(data)?;
        Ok(())
    }
}

impl<R: Read + ?Sized> ReadPascalExt for R {}
impl<W: Write + ?Sized> WritePascalExt for W {}

#[cfg(test)]
mod tests {
    use crate::io_util::{ReadPascalExt, WritePascalExt};
    use bytes::{Buf, BufMut, BytesMut};

    #[test]
    fn test_write_pstring() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pstring("hello").unwrap();
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
    fn test_write_pstring_too_long() {
        let buf = BytesMut::new();
        let mut w = buf.writer();
        assert!(matches!(w.write_pstring(&"x".repeat(256)), Err(_)));
    }

    #[test]
    fn test_read_pstring() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pstring("hello").unwrap();
            buf = w.into_inner();
        }

        let mut r = buf.reader();
        assert_eq!(r.read_pstring().unwrap(), "hello");
    }

    #[test]
    fn test_read_pstring_too_short() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pstring("hello").unwrap();
            buf = w.into_inner();
        }

        buf = buf.split_off(1);

        let mut r = buf.reader();
        assert!(matches!(r.read_pstring(), Err(_)));
    }

    #[test]
    fn test_write_pbuffer() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pbuffer(&[1, 2, 3, 4]).unwrap();
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
    fn test_write_pbuffer_too_long() {
        let buf = BytesMut::new();
        let mut w = buf.writer();
        assert!(matches!(
            w.write_pbuffer(&[1u8].repeat(u16::MAX as usize + 1)),
            Err(_)
        ));
    }

    #[test]
    fn test_read_pbuffer() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pbuffer(&[1, 2, 3, 4]).unwrap();
            buf = w.into_inner();
        }

        let mut r = buf.reader();
        assert_eq!(r.read_pbuffer(5000).unwrap(), [1, 2, 3, 4]);
    }

    #[test]
    fn test_read_pbuffer_too_short() {
        let mut buf = BytesMut::new();
        {
            let mut w = buf.writer();
            w.write_pbuffer(&[1, 2, 3, 4]).unwrap();
            buf = w.into_inner();
        }

        buf = buf.split_off(1);

        let mut r = buf.reader();
        assert!(matches!(r.read_pbuffer(500), Err(_)));
    }
}
