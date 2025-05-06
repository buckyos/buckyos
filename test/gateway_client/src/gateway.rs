use tokio::net::TcpListener;


async fn start_echo_server() {
    // run tcp echo server on 127.0.0.1:1009 for test
    let listener = TcpListener::bind("127.0.0.1:1009").await.unwrap();
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

fn start_echo_loop_with_socks5_to_etcd1() {
    tokio::spawn(async move {
        loop {
            let _ = crate::util::echo_with_socks5(1080, "etcd1:1008").await;

            // sleep 5s
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });
}

fn start_echo_loop_with_forward_to_etcd1() {
    tokio::spawn(async move {
        loop {
            let _ = crate::util::echo_with_forward("127.0.0.1:1088").await;

            // sleep 5s
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });
}

pub async fn run() {
    start_echo_server().await;

    // client -> gateway -> etcd1 -> upstream
    // start_echo_loop_with_socks5_to_etcd1();

    // client -> gateway -> etcd1 -> upstream
    start_echo_loop_with_forward_to_etcd1();

    // wait infinite
    tokio::signal::ctrl_c().await.unwrap();
}
