use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf, Result, SeekFrom};

pub trait MtreeReadSeek: AsyncRead + AsyncSeek + Send + Sync + Unpin {}

// Blanket implementation for any type that implements both traits
impl<T: AsyncRead + AsyncSeek + Send + Sync + Unpin> MtreeReadSeek for T {}

pub trait MtreeWriteSeek: AsyncWrite + AsyncSeek + Send + Sync + Unpin {}
// Blanket implementation for any type that implements both traits
impl<T: AsyncWrite + AsyncSeek + Send + Sync + Unpin> MtreeWriteSeek for T {}

// Use this struct to wrap a MtreeReadSeek and add an offset to the read position
pub struct MtreeReadSeekWithOffset<T: MtreeReadSeek> {
    inner: T,
    offset: u64,
}

impl<T: MtreeReadSeek> MtreeReadSeekWithOffset<T> {
    pub fn new(inner: T, offset: u64) -> Self {
        Self { inner, offset }
    }
}

impl<T: MtreeReadSeek> AsyncRead for MtreeReadSeekWithOffset<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<T: MtreeReadSeek> AsyncSeek for MtreeReadSeekWithOffset<T> {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> tokio::io::Result<()> {
        let position = match position {
            SeekFrom::Start(offset) => SeekFrom::Start(offset + self.as_mut().offset),
            _ => position,
        };
        Pin::new(&mut self.as_mut().inner).start_seek(position)
    }

    fn poll_complete(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<u64>> {
        Pin::new(&mut self.get_mut().inner).poll_complete(cx)
    }
}

impl<T: MtreeReadSeek> Unpin for MtreeReadSeekWithOffset<T> {}

pub struct MtreeWriteSeekWithOffset<T: MtreeWriteSeek> {
    inner: T,
    offset: u64,
}

impl<T: MtreeWriteSeek> MtreeWriteSeekWithOffset<T> {
    pub fn new(inner: T, offset: u64) -> Self {
        Self { inner, offset }
    }
}

impl<T: MtreeWriteSeek> AsyncWrite for MtreeWriteSeekWithOffset<T> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<T: MtreeWriteSeek> AsyncSeek for MtreeWriteSeekWithOffset<T> {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> tokio::io::Result<()> {
        let position = match position {
            SeekFrom::Start(offset) => SeekFrom::Start(offset + self.as_mut().offset),
            _ => position,
        };
        Pin::new(&mut self.as_mut().inner).start_seek(position)
    }

    fn poll_complete(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<u64>> {
        Pin::new(&mut self.get_mut().inner).poll_complete(cx)
    }
}

impl<T: MtreeWriteSeek> Unpin for MtreeWriteSeekWithOffset<T> {}

use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct SharedBuffer {
    data: Vec<u8>,
    pos: usize,
}

impl SharedBuffer {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            pos: 0,
        }
    }

    pub fn with_size(size: usize) -> Self {
        Self {
            data: vec![0u8; size],
            pos: 0,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data: Vec::with_capacity(capacity),
            pos: 0,
        }
    }
}

#[derive(Clone)]
pub struct MtreeReadWriteSeekWithSharedBuffer {
    shared_buf: Arc<Mutex<SharedBuffer>>,
}

impl MtreeReadWriteSeekWithSharedBuffer {
    pub fn new(shared_buf: SharedBuffer) -> Self {
        Self { 
            shared_buf: Arc::new(Mutex::new(shared_buf))
        }
    }

    pub fn buffer(&self) -> Arc<Mutex<SharedBuffer>> {
        self.shared_buf.clone()
    }
}

impl AsyncWrite for MtreeReadWriteSeekWithSharedBuffer {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut shared_buf = self.shared_buf.lock().unwrap();
        let end = shared_buf.pos + buf.len();

        // Check if there is enough space
        if end > shared_buf.data.len() {
            return Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "not enough space in the buffer",
            )));
        }

        // Write the data at the current position
        let pos = shared_buf.pos;
        shared_buf.data[pos..end].copy_from_slice(buf);
        shared_buf.pos = end;

        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl AsyncSeek for MtreeReadWriteSeekWithSharedBuffer {
    fn start_seek(mut self: Pin<&mut Self>, position: SeekFrom) -> std::io::Result<()> {
        let mut shared_buf = self.shared_buf.lock().unwrap();
        match position {
            SeekFrom::Start(offset) => {
                shared_buf.pos = offset as usize;
            }
            SeekFrom::Current(offset) => {
                shared_buf.pos = (shared_buf.pos as i64 + offset) as usize;
            }
            SeekFrom::End(offset) => {
                let len = shared_buf.data.len() as i64;
                shared_buf.pos = (len + offset) as usize;
            }
        }
        Ok(())
    }

    fn poll_complete(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
        let shared_buf = self.shared_buf.lock().unwrap();
        Poll::Ready(Ok(shared_buf.pos as u64))
    }
}

impl AsyncRead for MtreeReadWriteSeekWithSharedBuffer {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let mut shared_buf = self.shared_buf.lock().unwrap();
        let len: usize = std::cmp::min(shared_buf.pos + buf.remaining(), shared_buf.data.len());
        buf.put_slice(&shared_buf.data[shared_buf.pos..len]);
        shared_buf.pos = len;

        Poll::Ready(Ok(()))
    }
}
