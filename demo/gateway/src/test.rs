use crate::gateway::Gateway;

use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_socks::tcp::Socks5Stream;
use tokio::io::{AsyncWriteExt, AsyncReadExt};

const ETCD1_CONFIG: &str = r#"
{
    "config": {
        "device_id": "etcd1",
        "addr_type": "lan",
        "tunnel_server_port": 23559
    },
    "known_device": [
        {
            "id": "gateway",
            "addr": "192.168.100.110",
            "addr_type": "wan"
        }
    ],
    "service": [{
        "block": "upstream",
        "type": "tcp",
        "addr": "127.0.0.1",
        "port": 1008
    }]
}
"#;

const GATEWAY_CONFIG: &str = r#"
{
    "config": {
        "device_id": "gateway",
        "addr_type": "wan"
    },
    "known_device": [
        {
            "id": "etcd1",
            "addr": "192.168.100.110",
            "port": 23559,
            "addr_type": "wan"
        }
    ],
    "service": [{
        "block": "proxy",
        "addr": "127.0.0.1",
        "port": 1080,
        "type": "socks5"
    }]
}
"#;

async fn start_etcd1() {
    let config = serde_json::from_str(ETCD1_CONFIG).unwrap();
    let etcd1 = Gateway::load(&config).unwrap();
    etcd1.start().await.unwrap();

    // run tcp echo server on 127.0.0.1:1008 for test
    let listener = TcpListener::bind("127.0.0.1:1008").await.unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut reader, mut writer) = socket.split();
                tokio::io::copy(&mut reader, &mut writer).await.unwrap();
            });
        }
    });
}

async fn start_gateway() {
    let config = serde_json::from_str(GATEWAY_CONFIG).unwrap();
    let gateway = Gateway::load(&config).unwrap();
    gateway.start().await.unwrap();
}

pub async fn test_with_socks5() {
    let proxy_addr = "127.0.0.1:1080".parse::<SocketAddr>().unwrap();
    let target_addr = "etcd1:1008";

    let stream = Socks5Stream::connect(proxy_addr, target_addr)
        .await
        .unwrap();
    info!("Connect to socks5 proxy success! proxy={}, target={}", proxy_addr, target_addr);
    
    let (mut reader, mut writer) = stream.into_inner().into_split();

    // write some bytes and then recv them back
    let data = b"hello world";

    tokio::spawn(async move {
        let mut buf = vec![0u8; data.len()];
        reader.read_exact(&mut buf).await.unwrap();
        assert_eq!(buf, data);

        info!("Read echo data success!");
    });

    writer.write_all(data).await.unwrap();
    info!("Write data success!");
}

#[tokio::test]
async fn test_main() {
    // init log
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();
    info!("Will run etcd1 and gateway...");

    start_gateway().await;
    start_etcd1().await;

    // sleep 5s
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    test_with_socks5().await;

    // sleep 5s
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
}
