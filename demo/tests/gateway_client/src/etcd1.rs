use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn start_echo_server() {
    // run tcp echo server on 127.0.0.1:1008 for test
    let listener = TcpListener::bind("127.0.0.1:1008").await.unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, addr) = listener.accept().await.unwrap();
            info!("New connection on echo server: {}", addr);

            tokio::spawn(async move {
                let (mut reader, mut writer) = socket.split();
                // tokio::io::copy(&mut reader, &mut writer).await.unwrap();

                let mut buffer = [0; 1024 * 10];

                loop {
                    // 从reader中读取数据
                    match reader.read(&mut buffer).await {
                        Ok(0) => {
                            // 连接已关闭
                            info!("Connection closed on echo server: {}", addr);
                            break;
                        }
                        Ok(n) => {
                            // 输出日志
                            info!("Read {} bytes on echo server: {}", n, addr);

                            // 将数据写入writer
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

fn start_echo_loop_with_socks5_to_gateway() {
    tokio::spawn(async move {
        loop {
            let _ = crate::util::echo_with_socks5(1081, "gateway:1009").await;

            // sleep 5s
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    });
}

pub async fn run() {
    start_echo_server().await;

    // client -> etcd1 -> gateway -> upstream
    // start_echo_loop_with_socks5_to_gateway();

    // wait infinite
    tokio::signal::ctrl_c().await.unwrap();
}
