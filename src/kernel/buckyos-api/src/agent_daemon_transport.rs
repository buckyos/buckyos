use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

pub fn agent_daemon_byte_stream<S>(socket: WebSocketStream<S>) -> DuplexStream
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (client, bridge) = tokio::io::duplex(128 * 1024);
    tokio::spawn(async move {
        let (mut read, mut write) = tokio::io::split(bridge);
        let (mut sink, mut source) = socket.split();
        let (pong_tx, mut pong_rx) = tokio::sync::mpsc::channel(8);
        {
            let outgoing = async {
                let mut buffer = vec![0_u8; 64 * 1024];
                loop {
                    tokio::select! {
                        result = read.read(&mut buffer) => match result {
                            Ok(0) | Err(_) => break,
                            Ok(size) => {
                                if sink.send(Message::Binary(buffer[..size].to_vec().into())).await.is_err() { break; }
                            }
                        },
                        pong = pong_rx.recv() => match pong {
                            Some(()) => if sink.flush().await.is_err() { break; },
                            None => break,
                        }
                    }
                }
            };
            let incoming = async {
                loop {
                    match source.next().await {
                        Some(Ok(Message::Binary(bytes))) if bytes.len() <= 1024 * 1024 => {
                            if write.write_all(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Ping(_))) => {
                            if pong_tx.send(()).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(Message::Pong(_))) => {}
                        _ => break,
                    }
                }
            };
            tokio::select! {
                _ = incoming => {},
                _ = outgoing => {},
            }
        }
        let _ = write.shutdown().await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), sink.close()).await;
    });
    client
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::Role;

    #[tokio::test]
    async fn binary_boundaries_and_ping_preserve_byte_stream() {
        let (left, right) = tokio::io::duplex(256 * 1024);
        let socket = WebSocketStream::from_raw_socket(left, Role::Client, None).await;
        let mut peer = WebSocketStream::from_raw_socket(right, Role::Server, None).await;
        let mut bytes = agent_daemon_byte_stream(socket);
        peer.send(Message::Binary(vec![1, 2].into())).await.unwrap();
        peer.send(Message::Ping(vec![5].into())).await.unwrap();
        peer.send(Message::Binary(vec![3, 4].into())).await.unwrap();
        let mut output = [0; 4];
        bytes.read_exact(&mut output).await.unwrap();
        assert_eq!(output, [1, 2, 3, 4]);
        assert!(matches!(
            peer.next().await.unwrap().unwrap(),
            Message::Pong(_)
        ));
        bytes.write_all(b"reply").await.unwrap();
        assert_eq!(
            peer.next().await.unwrap().unwrap(),
            Message::Binary(b"reply".to_vec().into())
        );
        peer.send(Message::Text("invalid".into())).await.unwrap();
        assert_eq!(bytes.read(&mut output).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn simultaneous_large_transfers_do_not_deadlock() {
        let (left, right) = tokio::io::duplex(32 * 1024);
        let left = agent_daemon_byte_stream(
            WebSocketStream::from_raw_socket(left, Role::Client, None).await,
        );
        let right = agent_daemon_byte_stream(
            WebSocketStream::from_raw_socket(right, Role::Server, None).await,
        );
        let (mut left_read, mut left_write) = tokio::io::split(left);
        let (mut right_read, mut right_write) = tokio::io::split(right);
        let payload = vec![42; 1024 * 1024];
        let mut a = vec![0; payload.len()];
        let mut b = vec![0; payload.len()];
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::try_join!(
                left_write.write_all(&payload),
                right_write.write_all(&payload),
                left_read.read_exact(&mut a),
                right_read.read_exact(&mut b)
            )
            .unwrap();
        })
        .await
        .unwrap();
        assert_eq!(a, payload);
        assert_eq!(b, payload);
    }
}
