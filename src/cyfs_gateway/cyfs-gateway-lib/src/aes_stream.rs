use aes::cipher::{KeyIvInit, StreamCipher};
use cipher::StreamCipherSeek;
use ctr::Ctr128BE;
use rand::thread_rng;

use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use futures::ready;
use log::*;
use rand::Rng;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

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
        let original_filled = buf.filled().len();
        ready!(Pin::new(&mut self.inner).poll_read(cx, buf))?;
        let newly_filled = &mut buf.filled_mut()[original_filled..];
        if !newly_filled.is_empty() {
            self.decrypt_cipher.apply_keystream(newly_filled);
        }
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for EncryptedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let write_nonce = thread_rng().gen::<u64>();
        let mut encrypted = buf.to_vec();
        let org_pos:usize = self.encrypt_cipher.current_pos();
        self.encrypt_cipher.apply_keystream(&mut encrypted);
        //info!("{} aes stream encrypted data: [{}-{}]", write_nonce, org_pos, org_pos+buf.len());
        
        match Pin::new(&mut self.inner).poll_write(cx, &encrypted) {
            Poll::Ready(Ok(written)) => {
                if written < encrypted.len() {
                    warn!("{} aes stream encrypted data partial write, expect:{} actual:{},seek_pos:{}", 
                        write_nonce,encrypted.len(), written,org_pos+written); 
                    self.encrypt_cipher.seek(org_pos+written);
                }
                //info!("{} aes stream encrypted data write OK: [{}-{}]", write_nonce, org_pos, org_pos+written);
                return Poll::Ready(Ok(written));
            },
            Poll::Ready(Err(e)) => {
                return Poll::Ready(Err(e));
            },
            Poll::Pending => {
                self.encrypt_cipher.seek(org_pos);
                return Poll::Pending;
            }
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