//! D2 (AI Native 余下任务 TODO §5): the WAL of `doc.sqlite` while a wish stage snapshot holds its read transaction
//! and the Workspace keeps committing, and after the snapshot is dropped. A probe, not an assertion of a limit.
//!
//!   cargo test -p aiworkspace-store --release --test wal_growth -- --ignored --nocapture --test-threads=1

mod common;
use aiworkspace_store::workspace::Workspace;
use common::*;
use serde_json::json;
use std::time::Instant;

const COMMITS: usize = 2000;
const STEP: usize = 250;
const ROWS: usize = 50;

fn wal_bytes(ws: &Workspace) -> u64 {
    std::fs::metadata(ws.dir.join("doc.sqlite-wal")).map(|m| m.len()).unwrap_or(0)
}

fn db_bytes(ws: &Workspace) -> u64 {
    std::fs::metadata(ws.dir.join("doc.sqlite")).map(|m| m.len()).unwrap_or(0)
}

fn mb(b: u64) -> f64 {
    b as f64 / 1_048_576.0
}

fn table(ws: &mut Workspace) {
    ok(ws, &alice(), json!([{ "op": "entity.create", "entity_id": "t", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "T",
        "payload": { "title_field_id": "title", "fields": [
            { "field_id": "title", "name": "Title", "type": "text" },
            { "field_id": "n", "name": "N", "type": "number" } ] } }]));
    let rows: Vec<_> = (0..ROWS).map(|i| json!({ "record_id": format!("r{i}"), "values": { "title": format!("row {i}"), "n": 0 } })).collect();
    ok(ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "t", "records": rows }]));
}

/// One small edit: one cell of one of the existing records (the Commit history still grows the database).
fn edit(ws: &mut Workspace, i: usize) {
    let rid = format!("r{}", i % ROWS);
    let rev = ws.read(&alice(), "t", Some(&json!({ "kind": "table_cell", "record_id": rid, "field_id": "n" }))).unwrap()["content"]["rev"].clone();
    ok(ws, &alice(), json!([{ "op": "table.set_values", "source_id": "t", "values": [{ "record_id": rid, "field_id": "n", "value": i, "expect": { "rev": rev } }] }]));
}

/// `PRAGMA wal_checkpoint(mode)` on a connection of its own: (busy, frames in the WAL, frames checkpointed).
fn checkpoint(ws: &Workspace, mode: &str) -> (i64, i64, i64) {
    let c = rusqlite::Connection::open(ws.dir.join("doc.sqlite")).unwrap();
    c.query_row(&format!("PRAGMA wal_checkpoint({mode})"), [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap()
}

fn series(hold: bool) {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "WAL", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    table(&mut ws);
    edit(&mut ws, 0);
    println!("\n== {} ==", if hold { "stage snapshot held" } else { "no snapshot (baseline)" });
    println!("before: db {:.2} MiB, wal {:.2} MiB", mb(db_bytes(&ws)), mb(wal_bytes(&ws)));
    let snap = hold.then(|| ws.wish_snapshot(&alice()).unwrap());
    let seen = |snap: &Option<aiworkspace_store::wish::snapshot::WishSnapshot>| {
        snap.as_ref().map(|s| s.conn().query_row("SELECT count(*) FROM commits", [], |r| r.get::<_, i64>(0)).unwrap_or(-1))
    };
    let at_open = seen(&snap);
    println!("commits | wal MiB | ms per commit (last {STEP})");
    let mut t = Instant::now();
    for i in 1..=COMMITS {
        edit(&mut ws, i);
        if i % STEP == 0 {
            println!("{i:7} | {:7.2} | {:.2}", mb(wal_bytes(&ws)), t.elapsed().as_secs_f64() * 1000.0 / STEP as f64);
            t = Instant::now();
        }
    }
    if let Some(s) = &snap {
        let q = Instant::now();
        let n = seen(&snap);
        println!("snapshot still sees head_seq {} (commits table {:?} at open, {:?} now; query {:.1} ms)", s.head_seq, at_open, n, q.elapsed().as_secs_f64() * 1000.0);
    }
    let peak = wal_bytes(&ws);
    drop(snap);
    println!("snapshot dropped: wal {:.2} MiB", mb(wal_bytes(&ws)));
    edit(&mut ws, COMMITS + 1);
    println!("after 1 more commit: wal {:.2} MiB", mb(wal_bytes(&ws)));
    for i in 0..STEP {
        edit(&mut ws, COMMITS + 2 + i);
    }
    println!("after {STEP} more commits: wal {:.2} MiB", mb(wal_bytes(&ws)));
    let (busy, log, done) = checkpoint(&ws, "PASSIVE");
    println!("PASSIVE checkpoint: busy {busy}, frames in wal {log}, checkpointed {done}; file {:.2} MiB", mb(wal_bytes(&ws)));
    let (busy, log, done) = checkpoint(&ws, "TRUNCATE");
    println!("TRUNCATE checkpoint: busy {busy}, frames {log}, checkpointed {done}; file {:.2} MiB (peak was {:.2})", mb(wal_bytes(&ws)), mb(peak));
    println!("db {:.2} MiB", mb(db_bytes(&ws)));
}

#[test]
#[ignore]
fn wal_while_a_stage_snapshot_is_open() {
    series(false);
    series(true);
}

/// What copying the document database costs (a fix that materializes the snapshot into a file of its own and ends
/// the read transaction at once): `VACUUM INTO` on a read-only connection, as the snapshot would open it.
#[test]
#[ignore]
fn copy_cost_of_a_materialized_snapshot() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "COPY", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    table(&mut ws);
    let mut total = ROWS;
    for target in [10_000usize, 50_000, 100_000] {
        while total < target {
            let rows: Vec<_> = (total..total + 5000)
                .map(|i| json!({ "record_id": format!("r{i}"), "values": { "title": format!("row {i} with some text to make it a little longer"), "n": i } }))
                .collect();
            ok(&mut ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "t", "records": rows }]));
            total += 5000;
        }
        checkpoint(&ws, "TRUNCATE");
        let out = env.dir.path().join(format!("copy-{target}.sqlite"));
        let c = rusqlite::Connection::open_with_flags(ws.dir.join("doc.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let t = Instant::now();
        c.execute("VACUUM INTO ?1", [out.to_str().unwrap()]).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let size = std::fs::metadata(&out).unwrap().len();
        println!("{target:7} records: db {:.1} MiB, VACUUM INTO {:.1} MiB in {ms:.0} ms", mb(db_bytes(&ws)), mb(size));
    }
}

/// The same hold on two plain connections, with the writer's `journal_size_limit` unset (as `open_db` leaves it) and
/// set: does the WAL file come back down by itself once the reader is gone?
#[test]
#[ignore]
fn journal_size_limit_after_the_reader_ends() {
    for limit in [None, Some(4 * 1024 * 1024i64)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.sqlite");
        let w = rusqlite::Connection::open(&path).unwrap();
        w.pragma_update(None, "journal_mode", "WAL").unwrap();
        w.pragma_update(None, "synchronous", "FULL").unwrap();
        if let Some(l) = limit {
            w.pragma_update(None, "journal_size_limit", l).unwrap();
        }
        w.execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); CREATE INDEX t_v ON t(v);").unwrap();
        let r = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        r.execute_batch("BEGIN DEFERRED").unwrap();
        let _: i64 = r.query_row("SELECT count(*) FROM t", [], |x| x.get(0)).unwrap();
        let wal = || std::fs::metadata(dir.path().join("d.sqlite-wal")).map(|m| m.len()).unwrap_or(0);
        for i in 0..2000 {
            w.execute("INSERT INTO t(v) VALUES (?1)", [format!("value {i} {}", "x".repeat(200))]).unwrap();
        }
        let held = wal();
        r.execute_batch("COMMIT").unwrap();
        for i in 0..1000 {
            w.execute("INSERT INTO t(v) VALUES (?1)", [format!("after {i}")]).unwrap();
        }
        println!("journal_size_limit {:?}: wal {:.2} MiB while held, {:.2} MiB 1000 commits after the reader ended", limit, mb(held), mb(wal()));
    }
}

/// A `VACUUM INTO` copy keeps the planner statistics (`sqlite_stat1`) the logical-table SQL depends on.
#[test]
#[ignore]
fn a_vacuum_copy_keeps_the_planner_statistics() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "STAT", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    table(&mut ws);
    let rows: Vec<_> = (ROWS..ROWS + 3000).map(|i| json!({ "record_id": format!("r{i}"), "values": { "title": format!("row {i}"), "n": i } })).collect();
    ok(&mut ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "t", "records": rows }]));
    let stat = |p: &std::path::Path| {
        let c = rusqlite::Connection::open_with_flags(p, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        c.query_row("SELECT group_concat(tbl || ':' || stat, ' | ') FROM sqlite_stat1", [], |r| r.get::<_, Option<String>>(0)).ok().flatten()
    };
    let out = env.dir.path().join("copy.sqlite");
    let c = rusqlite::Connection::open_with_flags(ws.dir.join("doc.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    c.execute("VACUUM INTO ?1", [out.to_str().unwrap()]).unwrap();
    println!("live: {:?}\ncopy: {:?}", stat(&ws.dir.join("doc.sqlite")), stat(&out));
}
