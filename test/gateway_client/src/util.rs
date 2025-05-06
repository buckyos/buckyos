use rand::Rng;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TestError {
    #[error("Test failed")]
    Failed,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Tokio socks error: {0}")]
    Socks(#[from] tokio_socks::Error),
}

pub type TestResult<T> = Result<T, TestError>;

fn generate_random_array() -> [u8; 1024] {
    let mut rng = rand::thread_rng();
    let mut data = [0u8; 1024];
    rng.fill(&mut data);
    data
}

async fn read_echo_data(mut reader: tokio::net::tcp::OwnedReadHalf, data: &[u8]) -> TestResult<()> {
    let mut buf = vec![0u8; data.len()];
    reader.read_exact(&mut buf).await.map_err(|e| {
        let msg = format!("Error reading from echo server: {}", e);
        error!("{}", msg);
        TestError::Io(e)
    })?;
    assert_eq!(buf, data);

    info!("Read echo data success!");

    Ok(())
}

pub async fn echo_with_socks5(proxy_port: u16, upstream_addr: &str) -> TestResult<()> {
    info!("Will echo via socks5 proxy, proxy_port={}, upstream_addr={}", proxy_port, upstream_addr);

    let proxy_addr = format!("127.0.0.1:{}", proxy_port)
        .parse::<SocketAddr>()
        .unwrap();

    let target_addr = upstream_addr;

    let stream = Socks5Stream::connect(proxy_addr, target_addr)
        .await
        .map_err(|e| {
            let msg = format!("Error connecting to socks5 proxy: {}", e);
            error!("{}", msg);
            TestError::Socks(e)
        })?;

    info!(
        "Connect to socks5 proxy success! proxy={}, target={}",
        proxy_addr, target_addr
    );

    let (reader, mut writer) = stream.into_inner().into_split();

    // write random bytes and then recv them back
    // let data = b"hello world";
    let data = generate_random_array();

    writer.write_all(&data).await.map_err(|e| {
        let msg = format!("Error writing to socks5 proxy: {}", e);
        error!("{}", msg);
        e
    })?;

    info!("Write echo data via socks5 proxy success!");

    // wait for read task with timeout
    match tokio::time::timeout(tokio::time::Duration::from_secs(5), read_echo_data(reader, &data))
        .await {
        Ok(Ok(_)) => {
            info!("Test socks5 echo success!");
            Ok(())
        }
        Ok(Err(e)) => {
            error!("Error in socks5 echo read task: {}", e);
            Err(TestError::Failed)
        }
        Err(e) => {
            let msg = format!("Timeout waiting for socks5 echo read task: {}", e);
            error!("{}", msg);
            Err(TestError::Timeout(msg.to_owned()))
        }
    }
}

pub async fn echo_with_forward(forward_addr: &str) -> TestResult<()> {
    info!("Will echo via forward proxy, forward_addr={}", forward_addr);

    let addr = forward_addr
        .parse::<SocketAddr>()
        .expect("Invalid forward address");

    let stream = TcpStream::connect(addr).await.map_err(|e| {
        let msg = format!("Error connecting to forward proxy: {}", e);
        error!("{}", msg);
        TestError::Io(e)
    })?;

    let (reader, mut writer) = stream.into_split();
    // write random bytes and then recv them back
    // let data = b"hello world";
    let data = generate_random_array();

    writer.write_all(&data).await.map_err(|e| {
        let msg = format!("Error writing to forward proxy: {}", e);
        error!("{}", msg);
        e
    })?;

    info!("Write echo data via forward proxy success!");

    writer.shutdown().await.map_err(|e| {
        let msg = format!("Error shutting down forward proxy writer: {}", e);
        error!("{}", msg);
        e
    })?;

    // wait for read task with timeout
    match tokio::time::timeout(tokio::time::Duration::from_secs(5), read_echo_data(reader, &data))
        .await
        {
            Ok(Ok(_)) => {
                info!("Test forward echo success!");
                Ok(())
            }
            Ok(Err(e)) => {
                error!("Error in forward echo read task: {}", e);
                Err(TestError::Failed)
            }
            Err(e) => {
                let msg = format!("Timeout waiting for forward echo read task: {}", e);
                error!("{}", msg);
                Err(TestError::Timeout(msg.to_owned()))
            }
        }
}
