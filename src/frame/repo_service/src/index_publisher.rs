use crate::crypto_utils::*;
use crate::def::*;
use crate::zone_info_helper::*;
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_service_data_dir};
use log::*;
use ndn_lib::*;
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite, SqlitePool};
use std::io::Read;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

pub struct IndexPublisher {}

impl IndexPublisher {
    async fn write_chunk(file_path: &PathBuf) -> RepoResult<ChunkId> {
        // calc local index chunk id
        let mut hasher = Sha256::new();
        let mut file = std::fs::File::open(file_path)?;
        let mut buffer = Vec::new();
        // index file is small, read all content at once
        file.read_to_end(&mut buffer)?;
        hasher.update(&buffer);
        let local_index_sha256 = hasher.finalize().to_vec();
        let chunk_id = ChunkId::from_sha256_result(&local_index_sha256);

        let named_mgr = NamedDataMgr::get_named_data_mgr_by_id(None)
            .await
            .ok_or_else(|| RepoError::NdnError("Failed to get repo named data mgr".to_string()))?;

        let named_mgr = named_mgr.lock().await;

        let (mut chunk_writer, progress_info) = named_mgr
            .open_chunk_writer(&chunk_id, buffer.len() as u64, 0)
            .await
            .map_err(|e| {
                error!("open_chunk_writer failed: {:?}", e);
                RepoError::NdnError(format!("open_chunk_writer failed: {:?}", e))
            })?;

        chunk_writer.write_all(&buffer).await.map_err(|e| {
            error!("write chunk failed: {:?}", e);
            RepoError::NdnError(format!("write chunk failed: {:?}", e))
        })?;

        named_mgr
            .complete_chunk_writer(&chunk_id)
            .await
            .map_err(|e| {
                error!("complete_chunk_writer failed: {:?}", e);
                RepoError::NdnError(format!("complete_chunk_writer failed: {:?}", e))
            })?;

        Ok(chunk_id)
    }

    async fn add_index_source_meta(index_source_meta: SourceMeta) -> RepoResult<()> {
        //打开LOCAL_INDEX_META_DB，如果不存在就创建
        let local_data_dir = get_buckyos_service_data_dir(SERVICE_NAME).join(LOCAL_INDEX_DATA);
        let index_meta_db_file = local_data_dir.join(LOCAL_INDEX_META_DB);

        let db_url = format!("sqlite://{}?mode=rwc", index_meta_db_file.to_string_lossy());
        let pool = SqlitePool::connect(&db_url).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS index_meta_db (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                did TEXT NOT NULL,
                name TEXT NOT NULL,
                version TEXT NOT NULL,
                chunk_id TEXT NOT NULL,
                sign TEXT NOT NULL,
                pub_time INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_chunk_id ON index_meta_db (chunk_id)")
            .execute(&pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_pub_time ON index_meta_db (pub_time)")
            .execute(&pool)
            .await?;

        //对version创建唯一索引
        sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS idx_version ON index_meta_db (version)")
            .execute(&pool)
            .await?;

        //insert index source meta
        let mut tx = pool.begin().await?;
        sqlx::query(
            "INSERT INTO index_meta_db (did, name, version, chunk_id, sign, pub_time) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&index_source_meta.did)
        .bind(&index_source_meta.name)
        .bind(&index_source_meta.version)
        .bind(&index_source_meta.chunk_id)
        .bind(&index_source_meta.sign)
        .bind(&index_source_meta.pub_time)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        info!(
            "add index source meta success, version:{}, chunk_id:{}, sign:{}, pub_time:{}",
            index_source_meta.version,
            index_source_meta.chunk_id,
            index_source_meta.sign,
            index_source_meta.pub_time
        );

        Ok(())
    }

    pub async fn pub_index(pem_file_path: &PathBuf, version: &str) -> RepoResult<()> {
        if !pem_file_path.exists() {
            return Err(RepoError::NotFound(format!(
                "Private key file {} not exists",
                pem_file_path.to_string_lossy()
            )));
        }
        let local_data_dir = get_buckyos_service_data_dir(SERVICE_NAME).join(LOCAL_INDEX_DATA);
        let local_index_file = local_data_dir.join(LOCAL_INDEX_DB);
        if !local_index_file.exists() {
            return Err(RepoError::NotFound(format!(
                "Local index file {} not exists",
                local_index_file.to_string_lossy()
            )));
        }

        let chunk_id = Self::write_chunk(&local_index_file).await?;
        let sign =
            sign_data(&pem_file_path.to_string_lossy(), &chunk_id.to_string()).map_err(|e| {
                error!("sign_data failed: {:?}", e);
                RepoError::SignError(format!("sign_data failed: {:?}", e))
            })?;

        // TODO fix did
        let index_source_meta = SourceMeta {
            did: ZoneInfoHelper::get_zone_did()?,
            name: ZoneInfoHelper::get_zone_name()?,
            version: version.to_string(),
            chunk_id: chunk_id.to_string(),
            sign,
            pub_time: buckyos_get_unix_timestamp() as i64,
        };

        Self::add_index_source_meta(index_source_meta).await?;

        Ok(())
    }

    pub async fn get_meta(version: Option<&str>) -> RepoResult<Option<SourceMeta>> {
        let local_data_dir = get_buckyos_service_data_dir(SERVICE_NAME).join(LOCAL_INDEX_DATA);
        let index_meta_db_file = local_data_dir.join(LOCAL_INDEX_META_DB);

        let db_url = format!("sqlite://{}", index_meta_db_file.to_string_lossy());
        let pool = SqlitePool::connect(&db_url).await?;

        let meta_info = if let Some(version) = version {
            sqlx::query_as::<_, SourceMeta>("SELECT * FROM index_meta_db WHERE version = ?")
                .bind(version)
                .fetch_optional(&pool)
                .await?
        } else {
            sqlx::query_as::<_, SourceMeta>(
                "SELECT * FROM index_meta_db ORDER BY pub_time DESC LIMIT 1",
            )
            .fetch_optional(&pool)
            .await?
        };

        info!("get_meta, version:{:?}, info: {:?}", version, meta_info);

        Ok(meta_info)
    }
}
