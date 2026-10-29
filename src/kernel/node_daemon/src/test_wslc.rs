use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn cli(args: &[&str]) -> String {
    let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    let output = run_command("docker", &args, None, None).await.unwrap();
    assert!(
        output.status.success(),
        "{}: {}",
        output.command_line,
        output.stderr
    );
    output.stdout
}

#[tokio::test]
#[ignore = "requires Windows WSLC with alpine:latest and ubuntu:24.04 already loaded"]
async fn windows_wslc_lifecycle() {
    assert!(wslc::program().unwrap().is_some());
    let name = format!("buckyos-app-wslc-test-{}", std::process::id());
    let volume = format!("buckyos-wslc-test-{}", std::process::id());
    let seed_volume = format!("{volume}-seed");
    let root = std::env::temp_dir().join(format!("buckyos wslc test {}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("source.txt"), "host-file").unwrap();
    let bind = format!("{}:/input:ro", root.display());
    let mount = format!("{volume}:/data");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let publish = format!("{port}:8080");
    cli(&["volume", "create", &volume]).await;
    assert!(cli(&["volume", "ls", "--format={{.Name}}"])
        .await
        .lines()
        .any(|entry| entry == volume));
    assert!(!cli(&["images", "-q", "alpine:latest"])
        .await
        .trim()
        .is_empty());
    assert!(cli(&["images", "-q", "buckyos-wslc-test-missing:never"])
        .await
        .trim()
        .is_empty());
    assert!(docker_image_listed(None, "alpine:latest").await.unwrap());
    assert!(crate::paios_images::PaiosDocker::new(None)
        .inspect("buckyos-wslc-test-missing:never")
        .await
        .unwrap()
        .is_none());
    let image_id = cli(&["image", "inspect", "--format={{.Id}}", "alpine:latest"]).await;
    assert!(image_id.starts_with("sha256:"));
    assert!(cli(&["images", "--quiet", "--no-trunc"])
        .await
        .lines()
        .any(|id| id.trim() == image_id.trim()));
    let tagged = format!("buckyos-wslc-test:{}", std::process::id());
    cli(&["tag", image_id.trim(), &tagged]).await;
    assert_eq!(
        cli(&["image", "inspect", "--format={{.Id}}", &tagged])
            .await
            .trim(),
        image_id.trim()
    );
    let digests = cli(&[
        "image",
        "inspect",
        "--format={{json .RepoDigests}}",
        &tagged,
    ])
    .await;
    serde_json::from_str::<Vec<String>>(&digests).unwrap();
    let mut args = vec![
        "run".to_string(),
        "-d".into(),
        "--name".into(),
        name.clone(),
    ];
    append_host_gateway_run_args(&mut args);
    assert!(!args.iter().any(|arg| arg == "--add-host"));
    assert_eq!(runtime_host_gateway().unwrap(), "host.wslc.internal");
    args.extend([
        "-p".into(),
        publish,
        "-v".into(),
        bind,
        "-v".into(),
        mount.clone(),
        "--label".into(),
        "buckyos.app_id=wslc-test".into(),
        "-e".into(),
        format!("BUCKYOS_HOST_GATEWAY={}", runtime_host_gateway().unwrap()),
        tagged.clone(),
        "sh".into(),
        "-c".into(),
        "printf ready > /data/index.html; while true; do printf 'HTTP/1.0 200 OK\r\nContent-Length: 5\r\n\r\nready' | nc -l -p 8080 -w 1; done".into(),
    ]);
    let output = run_command("docker", &args, None, None).await.unwrap();
    assert!(output.status.success(), "{}", output.stderr);
    let raw = cli(&["container", "inspect", "--format={{json .}}", &name]).await;
    let inspected: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(inspected["State"]["Running"], true);
    assert_eq!(inspected["Config"]["Labels"]["buckyos.app_id"], "wslc-test");
    assert!(cli(&[
        "container",
        "ls",
        "-a",
        "--format",
        "{{.Names}}",
        "--filter",
        &format!("name={name}")
    ])
    .await
    .lines()
    .any(|entry| entry == name));
    cli(&[
        "exec",
        &name,
        "sh",
        "-c",
        "test \"$(cat /input/source.txt)\" = host-file && ! touch /input/blocked",
    ])
    .await;
    let response = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)).await {
                stream
                    .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
                    .await
                    .unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                if response.ends_with("ready") {
                    break response;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert!(response.contains("200 OK"));
    cli(&["rm", "-f", &name]).await;
    assert_eq!(
        cli(&[
            "run",
            "--rm",
            "-v",
            &mount,
            "alpine:latest",
            "cat",
            "/data/index.html"
        ])
        .await,
        "ready"
    );
    let seed_mount = format!("{seed_volume}:/etc");
    let release = cli(&[
        "run",
        "--rm",
        "-v",
        &seed_mount,
        "alpine:latest",
        "cat",
        "/etc/alpine-release",
    ])
    .await;
    let seed_mount = format!("{seed_volume}:/data");
    assert_eq!(
        cli(&[
            "run",
            "--rm",
            "-v",
            &seed_mount,
            "alpine:latest",
            "cat",
            "/data/alpine-release"
        ])
        .await,
        release
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(b"buckyos-host-ready\n").await.unwrap();
    });
    let command = format!("cat < /dev/tcp/host.wslc.internal/{port}");
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        cli(&["run", "--rm", "ubuntu:24.04", "bash", "-c", &command]),
    )
    .await
    .unwrap();
    assert_eq!(result.trim(), "buckyos-host-ready");
    server.await.unwrap();
    cli(&["volume", "rm", &volume, &seed_volume]).await;
    cli(&["rmi", &tagged]).await;
    std::fs::remove_dir_all(root).unwrap();
}
