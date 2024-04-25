use std::{path::PathBuf, str::FromStr};
use std::path::Path;

pub struct Backup {
    url: String,
}

pub enum ListOffset {
    FromFirst(u32),
    FromLast(u32),
}

impl Backup {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }

    // TODO: 可能还需要一个公钥作为身份标识，否则可能被恶意应用篡改
    pub async fn begin_backup(
        &self,
        key: &str,
        version: u64,
        meta: &impl ToString,
        chunk_file_list: &[&std::path::Path],
    ) -> Result<Box<dyn tokio::io::AsyncWrite>, Box<dyn std::error::Error>> {
        // 1. put meta
        // 2. upload chunk files
        unimplemented!()
    }

    pub async fn query_versions<Meta: FromStr>(
        &self,
        key: &str,
        offset: ListOffset,
        limit: u32,
    ) -> Result<Vec<(u64, Meta)>, Box<dyn std::error::Error>> {
        unimplemented!()
    }

    pub async fn download_backup(
        &self,
        key: &str,
        version: u64,
        dir_path: &Path,
    ) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
        // 1. get chunk file list
        // 2. download chunk files to the dir_path
        unimplemented!()
    }
}

pub struct UploadStream {
    http_client: reqwest::Request,
}

impl UploadStream {
    pub fn new(key: String, version: String, meta: String) -> Self {
        unimplemented!()
    }
}

impl tokio::io::AsyncWrite for UploadStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        todo!()
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        todo!()
    }

    // 这里应该增加校验
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        todo!()
    }
}

pub struct DownloadStream {
    http_client: reqwest::Request,
}

impl DownloadStream {
    pub fn new(key: String, version: String) -> Self {
        unimplemented!()
    }
}

impl tokio::io::AsyncRead for DownloadStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        todo!()
    }
}
