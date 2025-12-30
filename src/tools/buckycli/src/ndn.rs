use std::path::PathBuf;
use ndn_lib::*;

pub async fn create_ndn_chunk(filepath: &str, target: &str) {
    let ndn_mgr_root_path = PathBuf::from(target);
    let file_path = PathBuf::from(filepath);
    let ndn_mgr = NamedDataMgr::get_named_data_mgr_by_path(ndn_mgr_root_path).await;
    if ndn_mgr.is_err() {
        println!("get ndn mgr at {} failed", target);
        return;
    }
    let _ndn_mgr = ndn_mgr.unwrap();
    let chunk_id = put_local_file_as_chunk(None,ChunkType::Mix256,&file_path,StoreMode::StoreInNamedMgr).await;
    if chunk_id.is_err() {
        println!("pub local file as chunk failed");
        return;
    }
    let chunk_id = chunk_id.unwrap();
    println!("pub local file as chunk success, chunk id: {}", chunk_id.to_string());
}
