use aes::cipher::{KeyIvInit, StreamCipher};
use ctr::Ctr128BE;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use futures::ready;
use log::*;

// 定义AES-256 CTR模式类型
pub type AesCtr = Ctr128BE<aes::Aes256>;

pub struct EncryptedStream<S> {
    inner: S,
    encrypt_cipher: AesCtr,  // 用于写入的cipher
    decrypt_cipher: AesCtr,  // 用于读取的cipher
}

impl<S> EncryptedStream<S> {
    pub fn new(inner: S, key: &[u8; 32], iv: &[u8; 16]) -> Self {
        Self {
            inner,
            encrypt_cipher: AesCtr::new(key.into(), iv.into()),
            decrypt_cipher: AesCtr::new(key.into(), iv.into()),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for EncryptedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // 读取新的数据
        let mut temp_buf = vec![0u8; buf.remaining()];
        let mut temp_read_buf = ReadBuf::new(&mut temp_buf);
        
        ready!(Pin::new(&mut self.inner).poll_read(cx, &mut temp_read_buf))?;
        
        if temp_read_buf.filled().is_empty() {
            return Poll::Ready(Ok(()));
        }

        // 解密数据
        let block = temp_read_buf.filled_mut();
        self.decrypt_cipher.apply_keystream(block);
        //info!("aes stream decrypted data: {}", block.len());
        
        buf.put_slice(block);
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for EncryptedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let mut encrypted = buf.to_vec();
        self.encrypt_cipher.apply_keystream(&mut encrypted); // 使用encrypt_cipher进行加密
        //info!("aes stream encrypted data: {}", encrypted.len());
        match ready!(Pin::new(&mut self.inner).poll_write(cx, &encrypted)) {
            Ok(n) => {
                if n < encrypted.len() {
                    Poll::Ready(Ok(n))
                } else {
                    Poll::Ready(Ok(buf.len()))
                }
            }
            Err(e) => Poll::Ready(Err(e))
        }
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


// pub async fn encrypted_copy_bidirectional<S1, S2>(
//     stream1: S1,
//     stream2: S2,
//     key: &[u8; 32]
// ) -> std::io::Result<()>
// where
//     S1: AsyncRead + AsyncWrite + Unpin,
//     S2: AsyncRead + AsyncWrite + Unpin,
// {
//     let mut encrypted_stream1 = EncryptedStream::new(stream1, key);
//     let mut encrypted_stream2 = EncryptedStream::new(stream2, key);
    
//     tokio::io::copy_bidirectional(&mut encrypted_stream1, &mut encrypted_stream2).await?;
    
//     Ok(())
// }