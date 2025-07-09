use super::def::{ndn_get_time_now, ObjectRelationType};
use crate::{NdnError, NdnResult, ObjId};
use rusqlite::{Connection, Result as SqlResult};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct NamedDataRelationItem {
    pub object_id: ObjId,
    pub target_id: ObjId,
    pub relation_type: ObjectRelationType,
}

pub(crate) struct NamedDataRelationDB {
    conn: Arc<Mutex<Connection>>,
}

impl NamedDataRelationDB {
    pub fn new_with_conn(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub async fn init(&self) -> NdnResult<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS object_relations (
                object_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                relation_type INTEGER NOT NULL,
                insert_time INTEGER NOT NULL,
                reference_count INTEGER NOT NULL,
                PRIMARY KEY (object_id, target_id, relation_type)
            )",
            [],
        )
        .map_err(|e| {
            let msg = format!("Failed to create object_relations table: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        Ok(())
    }

    // Put a relation into the database, if the relation already exists, it will be updated reference count
    pub async fn put_relation(&self, item: NamedDataRelationItem) -> NdnResult<()> {
        let conn = self.conn.lock().await;
        let insert_time = ndn_get_time_now();

        let result = conn.execute(
            "INSERT INTO object_relations (object_id, target_id, relation_type, insert_time, reference_count)
             VALUES (?1, ?2, ?3, ?4, 1)
             ON CONFLICT(object_id) DO UPDATE SET
                target_id = excluded.target_id,
                relation_type = excluded.relation_type,
                insert_time = excluded.insert_time,
                reference_count = object_relations.reference_count + 1",
            rusqlite::params![
                item.object_id.to_base32(),
                item.target_id.to_base32(),
                item.relation_type.as_i32(),
                insert_time
            ],
        );

        Ok(())
    }

    pub async fn get_relations(
        &self,
        object_id: &ObjId,
        relation_type: ObjectRelationType,
    ) -> NdnResult<Vec<ObjId>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare("SELECT object_id, target_id, relation_type FROM object_relations WHERE object_id = ?1 AND relation_type = ?2").map_err(|e| {
            let msg = format!("Failed to prepare statement: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        let mut rows = stmt
            .query(rusqlite::params![
                object_id.to_base32(),
                relation_type.as_i32()
            ])
            .map_err(|e| {
                let msg = format!("Failed to query relations: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().map_err(|e| {
            let msg = format!("Failed to iterate over rows: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })? {
            let target_id: String = row.get(1).map_err(|e| {
                let msg = format!("Failed to get target_id from row: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

            results.push(ObjId::new(&target_id)?);
        }

        Ok(results)
    }

    pub async fn get_relation_by_page(
        &self,
        object_id: &ObjId,
        page: u32,
        page_size: u32,
    ) -> NdnResult<Vec<ObjId>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn
            .prepare(
                "SELECT target_id FROM object_relations WHERE object_id = ?1 LIMIT ?2 OFFSET ?3",
            )
            .map_err(|e| {
                let msg = format!("Failed to prepare statement: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        let mut rows = stmt
            .query(rusqlite::params![
                object_id.to_base32(),
                page_size,
                page * page_size
            ])
            .map_err(|e| {
                let msg = format!("Failed to query relations by page: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().map_err(|e| {
            let msg = format!("Failed to iterate over rows: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })? {
            let target_id: String = row.get(0).map_err(|e| {
                let msg = format!("Failed to get target_id from row: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

            results.push(ObjId::new(&target_id)?);
        }

        Ok(results)
    }

    // Decrease a relation from the database, if the reference count is greater than 1, it will only decrease the reference count
    // If the reference count is 1, it will delete the relation
    pub async fn decrease_relation(
        &self,
        object_id: &ObjId,
        target_id: &ObjId,
        relation_type: ObjectRelationType,
    ) -> NdnResult<()> {
        let conn = self.conn.lock().await;

        let result = conn.execute(
            "UPDATE object_relations SET reference_count = reference_count - 1 WHERE object_id = ?1 AND target_id = ?2 AND relation_type = ?3 AND reference_count > 1",
            rusqlite::params![object_id.to_base32(), target_id.to_base32(), relation_type.as_i32()],
        ).map_err(|e| {
            let msg = format!("Failed to update relation: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        if result == 0 {
            // Target relation does not exist or reference count is 1, so try delete the relation 
            let ret = conn.execute(
                "DELETE FROM object_relations WHERE object_id = ?1 AND target_id = ?2 AND relation_type = ?3",
                rusqlite::params![object_id.to_base32(), target_id.to_base32(), relation_type.as_i32()],
            ).map_err(|e| {
                let msg = format!("Failed to delete relation: {}", e);
                error!("{}", msg);
                NdnError::DbError(msg)
            })?;

            if ret > 0 {
                info!("Deleted relation: {} -> {} with type {:?}", object_id.to_base32(), target_id.to_base32(), relation_type);
            } else {
                warn!("No relation found to delete: {} -> {} with type {:?}", object_id.to_base32(), target_id.to_base32(), relation_type);
            }
        } else {
            // Successfully decreased the reference count
        }

        Ok(())
    }

    // Remove a specific relation from the database, ignoring the reference count
    // This is useful when you want to remove a relation regardless of its reference count
    pub async fn remove_relation(
        &self,
        object_id: &ObjId,
        target_id: &ObjId,
        relation_type: ObjectRelationType,
    ) -> NdnResult<()> {
        let conn = self.conn.lock().await;

        let result = conn.execute(
            "DELETE FROM object_relations WHERE object_id = ?1 AND target_id = ?2 AND relation_type = ?3",
            rusqlite::params![object_id.to_base32(), target_id.to_base32(), relation_type.as_i32()],
        ).map_err(|e| {
            let msg = format!("Failed to delete relation: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        if result > 0 {
            info!("Deleted relation: {} -> {} with type {:?}", object_id.to_base32(), target_id.to_base32(), relation_type);
        } else {
            warn!("No relation found to delete: {} -> {} with type {:?}", object_id.to_base32(), target_id.to_base32(), relation_type);
        }

        Ok(())
    }

    // Remove all relations for a specific object, with the specified relation type
    pub async fn remove_object_relations(
        &self,
        object_id: &ObjId,
        relation_type: ObjectRelationType,
    ) -> NdnResult<()> {
        let conn = self.conn.lock().await;

        let result = conn.execute(
            "DELETE FROM object_relations WHERE object_id = ?1 AND relation_type = ?2",
            rusqlite::params![object_id.to_base32(), relation_type.as_i32()],
        ).map_err(|e| {
            let msg = format!("Failed to delete object relations: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        if result > 0 {
            info!("Deleted all relations for object: {}", object_id.to_base32());
        } else {
            warn!("No relations found to delete for object: {}", object_id.to_base32());
        }

        Ok(())
    }

    // Remove all relations for a specific object, regardless of relation type
    // This is useful when you want to clear all relations for an object
    pub async fn remove_object(
        &self,
        object_id: &ObjId,
    ) -> NdnResult<()> {
        let conn = self.conn.lock().await;

        let result = conn.execute(
            "DELETE FROM object_relations WHERE object_id = ?1",
            rusqlite::params![object_id.to_base32()],
        ).map_err(|e| {
            let msg = format!("Failed to delete object relations: {}", e);
            error!("{}", msg);
            NdnError::DbError(msg)
        })?;

        if result > 0 {
            info!("Deleted all relations for object: {}", object_id.to_base32());
        } else {
            warn!("No relations found to delete for object: {}", object_id.to_base32());
        }

        Ok(())
    }
}
