//! Scale probes (phase-one doc §8.3). Not capacity promises: three repeatable
//! fixtures from a fixed-seed generator, fed through commits, with timings
//! printed for the performance report.
//!
//!   cargo test -p aiworkspace-store --release --test perf -- --ignored --nocapture

mod common;
use aiworkspace_core::richtext::{self, BlockOp, Position};
use aiworkspace_store::workspace::Workspace;
use base64::Engine;
use common::*;
use serde_json::{json, Value};
use std::time::Instant;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*: fixed seed → identical fixture on every run
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn file_size(ws: &Workspace) -> u64 {
    ["doc.sqlite", "doc.sqlite-wal"].iter().map(|f| std::fs::metadata(ws.dir.join(f)).map(|m| m.len()).unwrap_or(0)).sum()
}

fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1).map(str::to_string)))
        .and_then(|kb| kb.parse::<f64>().ok())
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

const ROWS: usize = 10_000;
const FIELDS: usize = 20;

fn big_table(ws: &mut Workspace) {
    let mut fields = vec![json!({ "field_id": "title", "name": "标题", "type": "text", "required": true })];
    for i in 1..FIELDS {
        let f = match i % 5 {
            0 => json!({ "field_id": format!("f{i}"), "name": format!("数值{i}"), "type": "number" }),
            1 => json!({ "field_id": format!("f{i}"), "name": format!("金额{i}"), "type": "decimal", "scale": 2 }),
            2 => json!({ "field_id": format!("f{i}"), "name": format!("日期{i}"), "type": "date" }),
            3 => json!({ "field_id": format!("f{i}"), "name": format!("状态{i}"), "type": "select",
                         "options": [{ "option_id": "a", "label": "甲" }, { "option_id": "b", "label": "乙" }, { "option_id": "c", "label": "丙" }] }),
            _ => json!({ "field_id": format!("f{i}"), "name": format!("文本{i}"), "type": "text" }),
        };
        fields.push(f);
    }
    ok(ws, &alice(), json!([{ "op": "entity.create", "entity_id": "big", "type_id": "buckyos.table-source", "parent_id": "page-main",
        "order_key": "p", "payload": { "title_field_id": "title", "fields": fields } }]));
    let mut rng = Rng(0x5eed_1234);
    for chunk in 0..ROWS / 1000 {
        let records: Vec<Value> = (0..1000)
            .map(|k| {
                let n = chunk * 1000 + k;
                let mut values = serde_json::Map::new();
                values.insert("title".into(), json!(format!("第 {n} 条记录")));
                for i in 1..FIELDS {
                    let v = match i % 5 {
                        0 => json!(rng.below(100_000)),
                        1 => json!(format!("{}.{:02}", rng.below(100_000), rng.below(100))),
                        2 => json!(format!("2026-{:02}-{:02}", 1 + rng.below(12), 1 + rng.below(28))),
                        3 => json!(["a", "b", "c"][rng.below(3) as usize]),
                        _ => json!(format!("文本内容 {}", rng.below(1_000_000))),
                    };
                    values.insert(format!("f{i}"), v);
                }
                json!({ "record_id": format!("r{n:05}"), "values": values })
            })
            .collect();
        ok(ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "big", "records": records }]));
    }
}

#[test]
#[ignore]
fn scale_probes() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    println!("\n== fixture: {ROWS} records x {FIELDS} fields ==");
    let t = Instant::now();
    big_table(&mut ws);
    println!("load (10 commits of 1000 inserts)      {:>9.1} ms   db {:.1} MiB", ms(t), file_size(&ws) as f64 / 1048576.0);

    // single cell write: one record row, independent of table size
    let mut samples = Vec::new();
    for i in 0..200 {
        let rid = format!("r{:05}", i * 37);
        let rev = ws.read(&alice(), "big", Some(&json!({ "kind": "table_cell", "record_id": rid, "field_id": "f4" }))).unwrap()["content"]["rev"].as_u64().unwrap();
        let t = Instant::now();
        ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "big", "values": [{ "record_id": rid, "field_id": "f4", "value": format!("改 {i}"), "expect": { "rev": rev } }] }]));
        samples.push(ms(t));
        assert_eq!(ws.last_stats.records, 1, "a single cell write must touch exactly one record row");
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("single cell commit (synchronous=FULL)   p50 {:>6.2} ms  p95 {:>6.2} ms", samples[100], samples[190]);

    // 1000-row batch in one commit: time inside the writer's critical section
    let values: Vec<Value> = (0..1000).map(|i| json!({ "record_id": format!("r{:05}", i * 7), "field_id": "f5", "value": i, "expect": { "rev": 0 } })).collect();
    let values: Vec<Value> = values
        .into_iter()
        .map(|mut v| {
            let rid = v["record_id"].as_str().unwrap().to_string();
            let rev = ws.read(&alice(), "big", Some(&json!({ "kind": "table_cell", "record_id": rid, "field_id": "f5" }))).unwrap()["content"]["rev"].clone();
            v["expect"]["rev"] = rev;
            v
        })
        .collect();
    let t = Instant::now();
    let batch = ok(&mut ws, &alice(), json!([{ "op": "table.set_values", "source_id": "big", "values": values }]));
    println!("1000-row batch commit                  {:>9.1} ms   rows written {}", ms(t), ws.last_stats.records);
    let t = Instant::now();
    let req = json!({ "epoch": ws.epoch, "commit_id": batch["commit_id"], "idempotency_key": "undo-batch" });
    assert_eq!(ws.undo(&alice(), &req)["status"], "accepted");
    println!("undo of the 1000-row batch             {:>9.1} ms", ms(t));

    // paged query: baseline is scan + filter + sort in Rust
    let q = json!({ "source_id": "big", "limit": 100, "filter": { "op": "cmp", "field_id": "f3", "operator": "eq", "value": "b" },
                    "sorts": [{ "field_id": "f1", "direction": "desc" }] });
    let t = Instant::now();
    let p1 = ws.query(&alice(), &q, &env.svc.sources).unwrap();
    let first = ms(t);
    let mut q2 = q.clone();
    q2["cursor"] = p1["next_cursor"].clone();
    let t = Instant::now();
    ws.query(&alice(), &q2, &env.svc.sources).unwrap();
    println!("query filter+sort, page of 100         first {:>6.1} ms  next {:>6.1} ms   ({} matches)", first, ms(t), p1["total"]);
    let t = Instant::now();
    ws.query(&alice(), &json!({ "source_id": "big", "limit": 100 }), &env.svc.sources).unwrap();
    println!("query unfiltered, page of 100          {:>9.1} ms", ms(t));

    let t = Instant::now();
    let cp = ws.checkpoint(&alice()).unwrap();
    println!("checkpoint (materialize all)           {:>9.1} ms", ms(t));
    let t = Instant::now();
    assert_eq!(ws.checkpoint(&alice()).unwrap().content_root, cp.content_root);
    println!("checkpoint again (cached objects)      {:>9.1} ms", ms(t));
    let t = Instant::now();
    let ex = ws.export(&alice(), "share", true).unwrap();
    let pkg = std::fs::metadata(ex["path"].as_str().unwrap()).unwrap().len();
    println!("export share package                   {:>9.1} ms   {:.1} MiB", ms(t), pkg as f64 / 1048576.0);
    let t = Instant::now();
    let rep = ws.replica_bootstrap(&alice()).unwrap();
    println!("replica bootstrap                      {:>9.1} ms   {:.1} MiB", ms(t), std::fs::metadata(rep["path"].as_str().unwrap()).unwrap().len() as f64 / 1048576.0);

    println!("\n== fixture: rich text, ~1000 blocks / ~100k chars ==");
    let blocks: Vec<Value> = (0..1000)
        .map(|i| json!({ "type": "paragraph", "attrs": { "block_id": format!("b{i:04}") },
                         "content": [{ "type": "text", "text": format!("第 {i} 段。{}", "这是一段用于容量探测的正文内容，包含中文与 ASCII mixed text。".repeat(2)) }] }))
        .collect();
    let chars: usize = blocks.iter().map(|b| b["content"][0]["text"].as_str().unwrap().chars().count()).sum();
    let t = Instant::now();
    ok(&mut ws, &alice(), json!([{ "op": "entity.create", "entity_id": "bigdoc", "type_id": "buckyos.richtext", "parent_id": "page-main",
        "order_key": "q", "payload": { "content": { "type": "doc", "content": blocks } } }]));
    println!("create ({chars} chars)                  {:>9.1} ms", ms(t));
    let st = ws.get_collab_state(&alice(), "bigdoc").unwrap();
    let client = richtext::load_doc(&base64::engine::general_purpose::STANDARD.decode(st["snapshot"].as_str().unwrap()).unwrap(), std::iter::empty()).unwrap();
    client.set_peer_id(99).unwrap();
    let lineage = st["lineage_id"].as_str().unwrap().to_string();
    let mut samples = Vec::new();
    for i in 0..100 {
        // an editor keystroke batch: one small CRDT update per commit
        let vv = client.oplog_vv();
        richtext::apply_block_ops(&client, &[BlockOp::Insert { position: Position::After(format!("b{:04}", i * 9)),
            blocks: vec![json!({ "type": "paragraph", "attrs": { "block_id": format!("k{i}") }, "content": [{ "type": "text", "text": "输入" }] })] }]).unwrap();
        let update = richtext::export_updates(&client, &vv).unwrap();
        let t = Instant::now();
        ok(&mut ws, &alice(), json!([{ "op": "richtext.apply_update", "entity_id": "bigdoc", "lineage_id": lineage,
            "update": base64::engine::general_purpose::STANDARD.encode(update) }]));
        samples.push(ms(t));
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("apply_update commit                     p50 {:>6.2} ms  p95 {:>6.2} ms", samples[50], samples[95]);
    let idx = ws.read(&alice(), "bigdoc", None).unwrap()["content"]["blocks"]["b0500"]["hash"].clone();
    let t = Instant::now();
    ok(&mut ws, &alice(), json!([{ "op": "richtext.replace_block", "entity_id": "bigdoc", "block_id": "b0500", "expect": { "hash": idx },
        "node": { "type": "paragraph", "attrs": { "block_id": "b0500" }, "content": [{ "type": "text", "text": "块级替换" }] } }]));
    println!("block-level replace commit             {:>9.1} ms", ms(t));

    println!("\n== fixture: 1000 entities (groups, cells, records) ==");
    let t = Instant::now();
    let mut ops = Vec::new();
    let mut key = None::<String>;
    for g in 0..20 {
        ops.push(json!({ "op": "entity.create", "entity_id": format!("g{g}"), "type_id": "buckyos.container", "parent_id": "page-main",
                         "order_key": format!("r{g:02}x"), "payload": { "kind": "group", "layout": { "mode": "free" } } }));
        for c in 0..49 {
            let k = aiworkspace_core::order_key::order_key_between(key.as_deref(), None).unwrap();
            ops.push(json!({ "op": "entity.create", "entity_id": format!("g{g}c{c}"), "type_id": "buckyos.cell", "parent_id": format!("g{g}"),
                             "order_key": k, "placement": { "x": c * 10, "y": g * 10, "w": 200, "h": 120, "z": 0 },
                             "payload": { "source_ref": { "entity_id": "tasks" }, "view": { "type": "table" }, "title": format!("视图 {g}-{c}") } }));
            key = Some(k);
        }
        key = None;
    }
    ok(&mut ws, &alice(), Value::Array(ops));
    println!("create 1000 entities in one commit     {:>9.1} ms", ms(t));
    let t = Instant::now();
    let outline = ws.outline(&alice()).unwrap();
    println!("outline of {} entities               {:>9.1} ms", outline["entities"].as_array().unwrap().len(), ms(t));
    let t = Instant::now();
    assert_eq!(ws.verify_refs().unwrap()["ok"], true);
    println!("reference index full rebuild + compare {:>9.1} ms", ms(t));
    // deleting a source referenced by ~980 cells is refused with the referrer list
    let life = ws.read(&alice(), "tasks", None).unwrap()["life_rev"].clone();
    let t = Instant::now();
    let r = commit(&mut ws, &alice(), json!([{ "op": "entity.delete", "entity_id": "tasks", "subtree": { "delete": ["task-42-details"] }, "expect": { "rev": life } }]));
    println!("delete referenced source (refused)     {:>9.1} ms   {}", ms(t), code(&r));

    let head = ws.head_seq;
    let size = file_size(&ws);
    drop(ws);
    env.svc.close(&id);
    let t = Instant::now();
    let h = env.svc.workspace(&id).unwrap();
    let ws = h.lock().unwrap();
    let open = ms(t);
    let t = Instant::now();
    ws.read(&alice(), "bigdoc", None).unwrap();
    ws.get_collab_state(&alice(), "bigdoc").unwrap();
    println!("\nreopen workspace                       {:>9.1} ms   first rich text load {:.1} ms", open, ms(t));
    println!("head_seq {head}, doc.sqlite(+wal) {:.1} MiB, peak RSS {:.0} MiB", size as f64 / 1048576.0, rss_mb());
}
