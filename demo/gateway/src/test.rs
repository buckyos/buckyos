use crate::gateway::Gateway;

const ETCD1_CONFIG: &str = r#"
{
    "config": {
        "device_id": "etcd1",
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
        "device_id": "gateway"
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

async fn run_etcd1() {
    let config = serde_json::from_str(ETCD1_CONFIG).unwrap();
    let etcd1 = Gateway::load(&config).unwrap();
    etcd1.start().await.unwrap();

    let config = serde_json::from_str(GATEWAY_CONFIG).unwrap();
    let gateway = Gateway::load(&config).unwrap();
    gateway.start().await.unwrap();
}

pub async fn main() {
    run_etcd1().await;
}

#[tokio::test]
async fn test_main() {
    // init log
    std::env::set_var("RUST_LOG", "info");
    env_logger::init();
    info!("Will run etcd1 and gateway...");

    run_etcd1().await;
}
