#![cfg(test)]

use std::{
    cmp,
    io::Error,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub struct TestStream {
    data: Vec<u8>,
    read: usize,
    pub chunk_size: Option<usize>,
    pub written: Vec<u8>,
}

impl TestStream {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            read: 0,
            chunk_size: None,
            written: Vec::new(),
        }
    }
}

impl AsyncRead for TestStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let to_write = match self.chunk_size {
            Some(chunk_size) => cmp::min(self.data.len(), chunk_size),
            None => self.data.len(),
        };
        buf.put_slice(&self.data[0..to_write]);
        self.read += to_write;
        self.data = self.data.split_off(to_write);

        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for TestStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        self.written.extend_from_slice(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }
}

pub struct StreamBuilder {
    stream: TestStream,
}

impl StreamBuilder {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            stream: TestStream::new(data),
        }
    }

    pub fn chunked(&mut self, chunk_size: usize) -> &Self {
        self.stream.chunk_size = Some(chunk_size);
        self
    }

    pub fn build(self) -> TestStream {
        self.stream
    }
}
