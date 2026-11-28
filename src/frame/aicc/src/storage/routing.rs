//! The user layer of the usecase table, routing groups and their App
//! bindings (doc/aicc/aicc_router.md §11–§12). Subjects are written
//! `user:<user_id>`, binding scopes `app:<app_id>`.

use super::policy::new_policy_id;
use super::*;
use buckyos_api::{AiccRoutingBinding, AiccRoutingGroup, AiccUsecaseRoutes};

fn group_from_row(row: &AnyRow) -> StorageResult<(String, AiccRoutingGroup)> {
    let content: String = row.try_get("content_json")?;
    Ok((
        row.try_get("owner")?,
        AiccRoutingGroup {
            group_id: row.try_get("group_id")?,
            name: row.try_get("name")?,
            usecases: serde_json::from_str(&content)?,
            version: from_i64(row.try_get("version")?)?,
            updated_at_ms: row.try_get("updated_at_ms")?,
            app_ids: Vec::new(),
        },
    ))
}

fn app_scope(app_id: &str) -> String {
    format!("app:{app_id}")
}

impl AiccStorage {
    /// The layer of `subject` and its version; version 0 when none is stored.
    pub(crate) async fn routing_layer(
        &self,
        subject: &str,
    ) -> StorageResult<(AiccUsecaseRoutes, u64)> {
        let sql = self.sql("SELECT content_json, version FROM aicc_routing_layer WHERE subject=?");
        let Some(row) = sqlx::query(&sql)
            .bind(subject)
            .fetch_optional(&self.pool)
            .await?
        else {
            return Ok((AiccUsecaseRoutes::new(), 0));
        };
        let content: String = row.try_get("content_json")?;
        Ok((
            serde_json::from_str(&content)?,
            from_i64(row.try_get("version")?)?,
        ))
    }

    /// Replace the layer of `subject` when it is still at
    /// `expected_version`; returns the new version.
    pub(crate) async fn put_routing_layer(
        &self,
        subject: &str,
        content: &AiccUsecaseRoutes,
        expected_version: u64,
        now_ms: i64,
    ) -> Result<u64, PolicyWriteError> {
        let mut tx = self.pool.begin().await?;
        let content_json = serde_json::to_string(content)?;
        let next = to_i64(expected_version.saturating_add(1))?;
        let changed = if expected_version == 0 {
            let insert = self.sql(
                "INSERT INTO aicc_routing_layer (subject,content_json,version,updated_at_ms)
                 VALUES (?,?,1,?) ON CONFLICT(subject) DO NOTHING",
            );
            sqlx::query(&insert)
                .bind(subject)
                .bind(&content_json)
                .bind(now_ms)
                .execute(&mut *tx)
                .await?
                .rows_affected()
        } else {
            let update = self.sql(
                "UPDATE aicc_routing_layer SET content_json=?, version=?, updated_at_ms=?
                 WHERE subject=? AND version=?",
            );
            sqlx::query(&update)
                .bind(&content_json)
                .bind(next)
                .bind(now_ms)
                .bind(subject)
                .bind(to_i64(expected_version)?)
                .execute(&mut *tx)
                .await?
                .rows_affected()
        };
        if changed == 0 {
            tx.rollback().await?;
            let (_, actual) = self.routing_layer(subject).await?;
            return Err(PolicyWriteError::VersionConflict {
                expected: expected_version,
                actual,
            });
        }
        tx.commit().await?;
        Ok(expected_version + 1)
    }

    /// The groups of `owner` with the Apps each is bound to.
    pub(crate) async fn routing_groups(&self, owner: &str) -> StorageResult<Vec<AiccRoutingGroup>> {
        let sql =
            self.sql("SELECT * FROM aicc_routing_group WHERE owner=? ORDER BY name, group_id");
        let rows = sqlx::query(&sql).bind(owner).fetch_all(&self.pool).await?;
        let bindings = self.routing_bindings(owner).await?;
        rows.iter()
            .map(|row| {
                let (_, mut group) = group_from_row(row)?;
                group.app_ids = bindings
                    .iter()
                    .filter(|binding| binding.group_id == group.group_id)
                    .map(|binding| binding.app_id.clone())
                    .collect();
                Ok(group)
            })
            .collect()
    }

    /// A group and its owner.
    pub(crate) async fn routing_group(
        &self,
        group_id: &str,
    ) -> StorageResult<Option<(String, AiccRoutingGroup)>> {
        let sql = self.sql("SELECT * FROM aicc_routing_group WHERE group_id=?");
        sqlx::query(&sql)
            .bind(group_id)
            .fetch_optional(&self.pool)
            .await?
            .as_ref()
            .map(group_from_row)
            .transpose()
    }

    pub(crate) async fn create_routing_group(
        &self,
        owner: &str,
        name: &str,
        content: &AiccUsecaseRoutes,
        now_ms: i64,
    ) -> StorageResult<AiccRoutingGroup> {
        let group = AiccRoutingGroup {
            group_id: new_policy_id("rg_", owner),
            name: name.to_string(),
            usecases: content.clone(),
            version: 1,
            updated_at_ms: now_ms,
            app_ids: Vec::new(),
        };
        let insert = self.sql(
            "INSERT INTO aicc_routing_group (group_id,owner,name,content_json,version,updated_at_ms)
             VALUES (?,?,?,?,1,?)",
        );
        sqlx::query(&insert)
            .bind(&group.group_id)
            .bind(owner)
            .bind(name)
            .bind(serde_json::to_string(content)?)
            .bind(now_ms)
            .execute(&self.pool)
            .await?;
        Ok(group)
    }

    /// Change a group of `owner` that is still at `expected_version`.
    pub(crate) async fn update_routing_group(
        &self,
        owner: &str,
        group_id: &str,
        expected_version: u64,
        name: Option<&str>,
        content: Option<&AiccUsecaseRoutes>,
        now_ms: i64,
    ) -> Result<AiccRoutingGroup, PolicyWriteError> {
        let Some((group_owner, mut group)) = self.routing_group(group_id).await? else {
            return Err(PolicyWriteError::NotFound);
        };
        if group_owner != owner {
            return Err(PolicyWriteError::NotFound);
        }
        if group.version != expected_version {
            return Err(PolicyWriteError::VersionConflict {
                expected: expected_version,
                actual: group.version,
            });
        }
        if let Some(name) = name {
            group.name = name.to_string();
        }
        if let Some(content) = content {
            group.usecases = content.clone();
        }
        let update = self.sql(
            "UPDATE aicc_routing_group SET name=?, content_json=?, version=?, updated_at_ms=?
             WHERE group_id=? AND owner=? AND version=?",
        );
        let changed = sqlx::query(&update)
            .bind(&group.name)
            .bind(serde_json::to_string(&group.usecases)?)
            .bind(to_i64(expected_version + 1)?)
            .bind(now_ms)
            .bind(group_id)
            .bind(owner)
            .bind(to_i64(expected_version)?)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if changed == 0 {
            let actual = self
                .routing_group(group_id)
                .await?
                .map(|(_, group)| group.version)
                .unwrap_or(0);
            return Err(PolicyWriteError::VersionConflict {
                expected: expected_version,
                actual,
            });
        }
        group.version = expected_version + 1;
        group.updated_at_ms = now_ms;
        group.app_ids = self
            .routing_bindings(owner)
            .await?
            .into_iter()
            .filter(|binding| binding.group_id == group_id)
            .map(|binding| binding.app_id)
            .collect();
        Ok(group)
    }

    /// Delete a group of `owner` and its App bindings.
    pub(crate) async fn delete_routing_group(
        &self,
        owner: &str,
        group_id: &str,
        expected_version: u64,
    ) -> Result<(), PolicyWriteError> {
        let mut tx = self.pool.begin().await?;
        let delete =
            self.sql("DELETE FROM aicc_routing_group WHERE group_id=? AND owner=? AND version=?");
        let changed = sqlx::query(&delete)
            .bind(group_id)
            .bind(owner)
            .bind(to_i64(expected_version)?)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if changed == 0 {
            tx.rollback().await?;
            return match self.routing_group(group_id).await? {
                Some((group_owner, group)) if group_owner == owner => {
                    Err(PolicyWriteError::VersionConflict {
                        expected: expected_version,
                        actual: group.version,
                    })
                }
                _ => Err(PolicyWriteError::NotFound),
            };
        }
        let unbind = self.sql("DELETE FROM aicc_routing_binding WHERE owner=? AND group_id=?");
        sqlx::query(&unbind)
            .bind(owner)
            .bind(group_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn routing_bindings(
        &self,
        owner: &str,
    ) -> StorageResult<Vec<AiccRoutingBinding>> {
        let sql = self
            .sql("SELECT scope, group_id FROM aicc_routing_binding WHERE owner=? ORDER BY scope");
        sqlx::query(&sql)
            .bind(owner)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(|row| {
                let scope: String = row.try_get("scope")?;
                Ok(AiccRoutingBinding {
                    app_id: scope.strip_prefix("app:").unwrap_or(&scope).to_string(),
                    group_id: row.try_get("group_id")?,
                })
            })
            .collect()
    }

    /// The group `owner` bound to `app_id`.
    pub(crate) async fn routing_binding(
        &self,
        owner: &str,
        app_id: &str,
    ) -> StorageResult<Option<String>> {
        let sql = self.sql("SELECT group_id FROM aicc_routing_binding WHERE owner=? AND scope=?");
        Ok(sqlx::query_scalar(&sql)
            .bind(owner)
            .bind(app_scope(app_id))
            .fetch_optional(&self.pool)
            .await?)
    }

    pub(crate) async fn set_routing_binding(
        &self,
        owner: &str,
        app_id: &str,
        group_id: &str,
        now_ms: i64,
    ) -> StorageResult<()> {
        let upsert = self.sql(
            "INSERT INTO aicc_routing_binding (owner,scope,group_id,updated_at_ms) VALUES (?,?,?,?)
             ON CONFLICT(owner, scope) DO UPDATE SET group_id=excluded.group_id,
             updated_at_ms=excluded.updated_at_ms",
        );
        sqlx::query(&upsert)
            .bind(owner)
            .bind(app_scope(app_id))
            .bind(group_id)
            .bind(now_ms)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(crate) async fn remove_routing_binding(
        &self,
        owner: &str,
        app_id: &str,
    ) -> StorageResult<bool> {
        let delete = self.sql("DELETE FROM aicc_routing_binding WHERE owner=? AND scope=?");
        Ok(sqlx::query(&delete)
            .bind(owner)
            .bind(app_scope(app_id))
            .execute(&self.pool)
            .await?
            .rows_affected()
            > 0)
    }
}

#[cfg(test)]
mod tests;
