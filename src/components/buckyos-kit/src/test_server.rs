use log::*; 
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use crate::init_logging;

pub async fn start_tcp_echo_server(bind_addr: &str) {
    // run tcp echo server on 127.0.0.1:1008 for test
    let listener = TcpListener::bind(bind_addr).await.unwrap();
    info!("Start tcp_echo_server on {}", bind_addr);
    tokio::spawn(async move {
        loop {
            let (mut socket, addr) = listener.accept().await.unwrap();
            info!("New connection on echo server: {}", addr);

            tokio::spawn(async move {
                let (mut reader, mut writer) = socket.split();
                // tokio::io::copy(&mut reader, &mut writer).await.unwrap();

                let mut buffer = [0; 1024 * 10];

                loop {

                    match reader.read(&mut buffer).await {
                        Ok(0) => {
                            info!("Connection closed on echo server: {}", addr);
                            break;
                        }
                        Ok(n) => {
                            info!("Read {} bytes on echo server: {}", n, addr);
                            if let Err(e) = writer.write_all(&buffer[..n]).await {
                                error!("Failed to write back to socket: {}, err = {:?}", addr, e);
                                break;
                            }
                        }
                        Err(e) => {
                            info!("Failed to read from socket: {} err = {:?}",  addr, e);
                            break;
                        }
                    }
                }
            });
        }
    });
}

pub async fn start_tcp_echo_client(server_addr: &str) {
    // connect to server_addr, send data, and read data
    let mut stream = TcpStream::connect(server_addr).await.unwrap();
    info!("Connected to tcp_echo_server at {}", server_addr);

    let data = b"hello world tcp";
    stream.write_all(data).await.unwrap();
    info!("Sent data: {:?}", data);

    let mut buffer = vec![0; data.len()];
    stream.read_exact(&mut buffer).await.unwrap();
    info!("Received data: {:?}", buffer);

    assert_eq!(data, &buffer[..]);
    info!("TCP echo test passed!");
}


pub async fn start_udp_echo_server(bind_addr: &str) {
    let socket = UdpSocket::bind(bind_addr).await.unwrap();
    info!("Start udp_echo_server on {}", bind_addr);

    let mut buffer = [0; 1024 * 10];
    tokio::spawn(async move {
        loop {
            let (n, addr) = socket.recv_from(&mut buffer).await.unwrap();
            info!("Received {} bytes from {}", n, addr);

            if let Err(e) = socket.send_to(&buffer[..n], &addr).await {
                error!("Failed to send data to {}: {:?}", addr, e);
            } else {
                info!("Sent {} bytes to {}", n, addr);
            }
        }
    });
}

pub async fn start_udp_echo_client(server_addr: &str) {
    // connect to server_addr, send data, and read data
    let socket = UdpSocket::bind("[::]:0").await.unwrap();
    info!("UDP client bound to {}", socket.local_addr().unwrap());

    let data = b"hello world udp";
    socket.send_to(data, server_addr).await.unwrap();
    info!("Sent data: {:?}", data);

    let mut buffer = vec![0; data.len()];
    let (n, addr) = socket.recv_from(&mut buffer).await.unwrap();
    info!("Received {} bytes from {}: {:?}", n, addr, &buffer[..n]);

    assert_eq!(data, &buffer[..n]);
    info!("UDP echo test passed!");
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_tcp_echo_server() {
        init_logging("test_tcp_echo_server");
        start_tcp_echo_server("[::]:10008").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        start_tcp_echo_client("127.0.0.1:10008").await;
        start_tcp_echo_client("[::1]:10008").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    #[tokio::test]
    async fn test_udp_echo_server() {
        init_logging("test_udp_echo_server");
        start_udp_echo_server("[::]:10009").await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        start_udp_echo_client("127.0.0.1:10009").await;
        start_udp_echo_client("[::1]:10009").await;
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}
