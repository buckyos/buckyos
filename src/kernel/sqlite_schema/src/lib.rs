//! Initialize a Desktop database and its format marker in one transaction.
//! A populated database with no marker must never acquire the current version on open.
use rusqlite::{Connection, Transaction, TransactionBehavior};

const STATE: &str = "SELECT user_version, schema_version, EXISTS(SELECT 1 FROM main.sqlite_schema)
                     FROM pragma_user_version, pragma_schema_version";

fn needs_initialization(
    version: i64,
    generation: i64,
    populated: bool,
    target: u32,
) -> Result<bool, String> {
    if version == i64::from(target) && populated {
        return Ok(false);
    }
    if version == 0 && generation == 0 && !populated {
        return Ok(true);
    }
    Err(format!(
        "unsupported SQLite schema {version} (schema_version={generation}, populated={populated}), expected {target}"
    ))
}

pub fn initialize(conn: &Connection, target: u32, ddl: &[&str]) -> rusqlite::Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let (version, generation, populated) =
        tx.query_row(STATE, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    if needs_initialization(version, generation, populated, target).map_err(|reason| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_SCHEMA),
            Some(reason),
        )
    })? {
        for statement in ddl {
            tx.execute_batch(statement)?;
        }
        tx.pragma_update(None, "user_version", target)?;
    }
    tx.commit()
}

/// Run creation DDL once, and idempotent service-spec DDL on every verified open.
/// Both run after checking the physical marker and within the same transaction.
#[cfg(feature = "sqlx")]
pub async fn initialize_pool(
    pool: &sqlx::AnyPool,
    target: u32,
    create: &[&str],
    on_open: &[&str],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let (version, generation, populated): (i64, i64, i64) =
        sqlx::query_as(STATE).fetch_one(&mut *tx).await?;
    let creating = needs_initialization(version, generation, populated != 0, target)
        .map_err(sqlx::Error::Protocol)?;
    let create = if creating { create } else { &[] };
    for statement in create.iter().chain(on_open) {
        sqlx::raw_sql(statement).execute(&mut *tx).await?;
    }
    if creating {
        sqlx::query(&format!("PRAGMA user_version = {target}"))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_is_atomic_and_reopening_never_adopts_an_unversioned_store() {
        let conn = Connection::open_in_memory().unwrap();
        let tables = |conn: &Connection| {
            conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
        };
        let ddl = "CREATE TABLE data (value TEXT)";
        assert!(initialize(&conn, 13, &[ddl, "broken SQL"]).is_err());
        assert_eq!(tables(&conn), 0);
        let dropped = Connection::open_in_memory().unwrap();
        dropped
            .execute_batch("CREATE TABLE data(n); DROP TABLE data")
            .unwrap();
        assert!(initialize(&dropped, 13, &[ddl]).is_err());
        assert_eq!(tables(&dropped), 0);
        initialize(&conn, 13, &[ddl]).unwrap();
        initialize(&conn, 13, &[ddl]).unwrap();
        for version in [0, 12, 14] {
            conn.pragma_update(None, "user_version", version).unwrap();
            assert!(initialize(&conn, 13, &["CREATE TABLE changed(n)"]).is_err());
            assert_eq!(tables(&conn), 1);
            assert_eq!(
                conn.pragma_query_value(None, "user_version", |r| r.get::<_, i32>(0))
                    .unwrap(),
                version
            );
        }
    }
}
