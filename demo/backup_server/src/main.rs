use async_std::fs::File;
use async_std::io::BufWriter;
use async_std::prelude::*;
use std::path::Path;
use tide::Request;

use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    save_path: String,
    interface: String,
    port: u16,
}

const HTTP_HEADER_KEY: &'static str = "BACKUP_KEY";
const HTTP_HEADER_VERSION: &'static str = "BACKUP_VERSION";
const HTTP_HEADER_METADATA: &'static str = "BACKUP_METADATA";

async fn upload_file(mut req: Request<String>) -> tide::Result {
    let save_path = req.state();

    // 解析 multipart 表单
    let key = match req.header(HTTP_HEADER_KEY) {
        Some(h) => h.to_string(),
        None => {
            return Err(tide::Error::from_str(
                tide::StatusCode::NotFound,
                "Key not found",
            ))
        }
    };

    let version = match req.header(HTTP_HEADER_VERSION) {
        Some(h) => h.to_string(),
        None => {
            return Err(tide::Error::from_str(
                tide::StatusCode::NotFound,
                "Version not found",
            ))
        }
    };

    let meta = req
        .header(HTTP_HEADER_METADATA)
        .map(|m| m.to_string())
        .unwrap_or("".to_string());

    let filename = format!("{}-{}-{}.tmp", key, version, 0);
    let path = Path::new(save_path).join(filename.as_str());
    let mut file = File::create(&path).await?;
    let mut writer = BufWriter::new(&mut file);

    // TODO 这里会一次接收整个body，可能会占用很大的内存
    loop {
        let body = req.body_bytes().await.map_err(|err| {
            log::error!("read stream {}-{} error: {}", key, version, err);
            err
        })?;

        if body.is_empty() {
            writer.flush().await.map_err(|err| {
                log::error!("flush stream {}-{} error: {}", key, version, err);
                err
            })?;
            break;
        }

        writer.write_all(body.as_slice()).await.map_err(|err| {
            log::error!("write stream {}-{} error: {}", key, version, err);
            err
        })?;
    }

    Ok(tide::Response::new(tide::StatusCode::Ok))
}

#[async_std::main]
async fn main() -> tide::Result<()> {
    let config_path = "./config.toml";
    let contents = tokio::fs::read_to_string(config_path)
        .await
        .map_err(|err| {
            log::error!("read config file failed: {}", err);
            err
        })
        .unwrap();

    // 解析 TOML 字符串到 Config
    let config: Config = toml::from_str(&contents)
        .map_err(|err| {
            log::error!("parse config file failed: {}", err);
            err
        })
        .unwrap();

    let mut app = tide::with_state(config.save_path.clone());

    app.at("/upload").post(upload_file);
    app.listen(format!("{}:{}", config.interface, config.port))
        .await
        .map_err(|err| {
            log::error!("listen failed: {}", err);
            err
        });

    Ok(())
}
