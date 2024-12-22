use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf, Result, SeekFrom};

pub trait MtreeReadSeek: AsyncRead + AsyncSeek + Unpin {}

// Blanket implementation for any type that implements both traits
impl<T: AsyncRead + AsyncSeek + Unpin> MtreeReadSeek for T {}

pub trait MtreeWriteSeek: AsyncWrite + AsyncSeek + Unpin {}
// Blanket implementation for any type that implements both traits
impl<T: AsyncWrite + AsyncSeek + Unpin> MtreeWriteSeek for T {}

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
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize>> {
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