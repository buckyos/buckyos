use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use aes::Aes256;
use aes::cipher::{BlockEncrypt, BlockDecrypt, KeyInit};
use std::pin::Pin;
use std::task::{Context, Poll};
use futures::ready;

struct EncryptedStream<S> {
    inner: S,
    cipher: Aes256,
    buffer: Vec<u8>,
    pos: usize,
}

impl<S> EncryptedStream<S> {
    fn new(inner: S, key: &[u8; 32]) -> Self {
        let cipher = Aes256::new(key.into());
        Self {
            inner,
            cipher,
            buffer: Vec::new(),
            pos: 0,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for EncryptedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // 如果buffer中还有数据,先读取buffer
        if self.pos < self.buffer.len() {
            let bytes_to_copy = std::cmp::min(buf.remaining(), self.buffer.len() - self.pos);
            buf.put_slice(&self.buffer[self.pos..self.pos + bytes_to_copy]);
            self.pos += bytes_to_copy;
            return Poll::Ready(Ok(()));
        }

        // 读取新的数据块
        let mut temp_buf = vec![0u8; 16]; // AES块大小
        let mut temp_read_buf = ReadBuf::new(&mut temp_buf);
        
        ready!(Pin::new(&mut self.inner).poll_read(cx, &mut temp_read_buf))?;
        
        if temp_read_buf.filled().is_empty() {
            return Poll::Ready(Ok(()));
        }

        // 解密数据
        let mut block = temp_read_buf.filled().to_vec();
        self.cipher.decrypt_block(block.as_mut_slice().into());
        
        // 存入buffer
        self.buffer = block;
        self.pos = 0;
        
        // 递归调用以读取解密后的数据
        self.poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for EncryptedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        // 确保数据块对齐
        let chunk_size = 16;
        let bytes_to_write = std::cmp::min(buf.len(), chunk_size);
        
        if bytes_to_write == 0 {
            return Poll::Ready(Ok(0));
        }

        let mut block = vec![0u8; chunk_size];
        block[..bytes_to_write].copy_from_slice(&buf[..bytes_to_write]);
        
        // 加密数据
        self.cipher.encrypt_block(block.as_mut_slice().into());
        
        // 写入加密后的数据
        ready!(Pin::new(&mut self.inner).poll_write(cx, &block))?;
        
        Poll::Ready(Ok(bytes_to_write))
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// 使用示例
async fn encrypted_copy_bidirectional<S1, S2>(
    stream1: S1,
    stream2: S2,
    key: &[u8; 32]
) -> std::io::Result<()>
where
    S1: AsyncRead + AsyncWrite + Unpin,
    S2: AsyncRead + AsyncWrite + Unpin,
{
    let mut encrypted_stream1 = EncryptedStream::new(stream1, key);
    let mut encrypted_stream2 = EncryptedStream::new(stream2, key);
    
    tokio::io::copy_bidirectional(&mut encrypted_stream1, &mut encrypted_stream2).await?;
    
    Ok(())
}