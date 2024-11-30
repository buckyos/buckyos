use crate::error::*;
use crate::source_manager::SourceMeta;
use crate::verifier::*;
use std::path::PathBuf;

pub async fn pull_remote_chunk(
    url: &str,
    author: &str,
    sign: &str,
    chunk_id: &str,
    chunk_mgr_id: &str,
) -> RepoResult<()> {
    //先验证
    Verifier::verify(author, chunk_id, sign).await?;
    //TODO 使用ndn下载
    unimplemented!("pull_remote_chunk")
}

pub async fn chunk_to_local_file(
    chunk_id: &str,
    chunk_mgr_id: &str,
    local_file: &PathBuf,
) -> RepoResult<()> {
    unimplemented!("chunk_to_local_file")
}

pub async fn get_remote_source_meta(url: &str) -> RepoResult<SourceMeta> {
    unimplemented!("get_remote_source_meta")
}
