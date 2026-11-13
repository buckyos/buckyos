//! T0-6 (notepads/aiws-core-api-unification-todo.md): the performance acceptance of the core API on a fixed-seed
//! Workspace of 100 000 records × 20 fields, a second table to join and a long document. Each read goes through the
//! store's entry points the service calls (`doc.query { sql | text }`, `doc.read { as: "body" }`); timings are
//! printed for the report and checked against the pass conditions of the design (aiws-core-api-design.md §12.2).
//!
//!   cargo test -p aiworkspace-store --release --test perf_core_api -- --ignored --nocapture

mod common;
use aiworkspace_core::sqlview::{compile_view, ident, Projection, ViewDef};
use aiworkspace_store::workspace::Workspace;
use common::*;
use serde_json::{json, Value};
use std::time::Instant;

const ROWS: usize = 100_000;
const FIELDS: usize = 20;
const DEPT: usize = 1000;
/// with a heading every 100 paragraphs: 4899 blocks, ~420 000 characters — near a document's limits (5000 blocks,
/// 500 000 UTF-16 units, schemas/richtext.basic.v1.json)
const PARAGRAPHS: usize = 4850;

// Pass conditions (aiws-core-api-design.md §12.2), on the reference machine of the baseline (i7-13700KF, release
// build): about 2.5–3× the baseline, so they catch a regression of the kind found here (a join without statistics,
// a page that rescans the table) and not the noise of a busy machine.
/// One SQL call over the 100 000-row table: a count, a filtered and sorted page (also deep), an aggregate, a LIKE
/// scan, 5000 rows of display columns, a join with a 1000-row table written with a materialized side.
const SQL_CALL_MS: f64 = 500.0;
/// What a Desktop without the replica runs to render a saved view: its page, its count and its groups.
const VIEW_RENDER_MS: f64 = 1000.0;
/// The time budget refuses within this much of its 2000 ms.
const TIME_BUDGET_SLACK_MS: f64 = 500.0;
/// A body page of the table, p95: 32 KiB pages and 256 KiB pages; no page above 3× the median (+5 ms), wherever it is.
const TABLE_PAGE_32K_MS: f64 = 20.0;
const TABLE_PAGE_256K_MS: f64 = 50.0;
/// Every page of a document at the size limit.
const DOC_PAGE_MS: f64 = 100.0;
/// All of the table's CSV, page after page, at either page size.
const TABLE_ALL_MS: f64 = 5000.0;
/// One text query call (the time budget is 1 s; a table is checked every 256 records), and one followed to the end
/// over the 100 000-row table or the whole Workspace.
const TEXT_CALL_MS: f64 = 1500.0;
const TEXT_ALL_MS: f64 = 5000.0;
/// `meta` of the table and of a view Block, `doc.find` by name.
const META_MS: f64 = 10.0;
/// `doc.read` of the table itself (its fields and record count; a Desktop without the replica reads it to render a view).
const TABLE_READ_MS: f64 = 50.0;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
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

/// `n` timed runs after one warm-up: (p50, max) in ms and the last result.
fn bench<T>(n: usize, mut f: impl FnMut() -> T) -> (f64, f64, T) {
    f();
    let mut samples = Vec::with_capacity(n);
    let mut last = None;
    for _ in 0..n {
        let t = Instant::now();
        last = Some(f());
        samples.push(ms(t));
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (samples[n / 2], samples[n - 1], last.unwrap())
}

struct Report(Vec<String>);
impl Report {
    fn row(&mut self, what: &str, value: String) {
        println!("{what:<58} {value}");
    }
    /// A pass condition: `value` ms at most `max` ms.
    fn at_most(&mut self, what: &str, value: f64, max: f64) {
        if value > max {
            self.0.push(format!("{what}: {value:.1} ms > {max} ms"));
        }
    }
}

fn load(ws: &mut Workspace) -> (String, String) {
    let mut fields = vec![json!({ "field_id": "title", "name": "标题", "type": "text", "required": true })];
    for i in 1..FIELDS {
        fields.push(match i % 5 {
            0 => json!({ "field_id": format!("f{i}"), "name": format!("数值{i}"), "type": "number" }),
            1 => json!({ "field_id": format!("f{i}"), "name": format!("金额{i}"), "type": "decimal", "scale": 2 }),
            2 => json!({ "field_id": format!("f{i}"), "name": format!("日期{i}"), "type": "date" }),
            3 => json!({ "field_id": format!("f{i}"), "name": format!("状态{i}"), "type": "select",
                         "options": [{ "option_id": "a", "label": "甲" }, { "option_id": "b", "label": "乙" }, { "option_id": "c", "label": "丙" }] }),
            _ => json!({ "field_id": format!("f{i}"), "name": format!("文本{i}"), "type": "text" }),
        });
    }
    ok(ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "big", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "p", "name": "大表",
          "payload": { "title_field_id": "title", "fields": fields } },
        { "op": "entity.create", "entity_id": "dept", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "q", "name": "部门",
          "payload": { "fields": [{ "field_id": "code", "name": "编号", "type": "number" }, { "field_id": "dname", "name": "名称", "type": "text" }] } },
    ]));
    let mut rng = Rng(0x5eed_1234);
    for chunk in 0..ROWS / 1000 {
        let records: Vec<Value> = (0..1000)
            .map(|k| {
                let n = chunk * 1000 + k;
                let mut values = serde_json::Map::new();
                // a token found only in the last record (the text query's worst case)
                let title = if n == ROWS - 1 { format!("第 {n} 条记录 末尾唯一词") } else { format!("第 {n} 条记录") };
                values.insert("title".into(), json!(title));
                for i in 1..FIELDS {
                    values.insert(format!("f{i}"), match i % 5 {
                        0 => json!(rng.below(100_000)),
                        1 => json!(format!("{}.{:02}", rng.below(100_000), rng.below(100))),
                        2 => json!(format!("2026-{:02}-{:02}", 1 + rng.below(12), 1 + rng.below(28))),
                        3 => json!(["a", "b", "c"][rng.below(3) as usize]),
                        _ => json!(format!("文本内容 {}", rng.below(1_000_000))),
                    });
                }
                json!({ "record_id": format!("r{n:06}"), "values": values })
            })
            .collect();
        ok(ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "big", "records": records }]));
    }
    let depts: Vec<Value> = (0..DEPT).map(|n| json!({ "record_id": format!("d{n:04}"), "values": { "code": n, "dname": format!("部门 {n}") } })).collect();
    ok(ws, &alice(), json!([{ "op": "table.insert_records", "source_id": "dept", "records": depts }]));

    // two saved views on a canvas: a filtered, sorted page with four fields, and a grouped one
    ok(ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "perf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "p",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "perf" } },
        { "op": "entity.create", "entity_id": "perf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "p",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "perf-content", "title": "性能" } },
        { "op": "entity.create", "entity_id": "view-open", "type_id": "buckyos.cell", "parent_id": "perf", "order_key": "a",
          "placement": { "x": 0, "y": 0, "w": 800, "h": 600 },
          "payload": { "source_ref": { "entity_id": "big" }, "view": { "type": "table" }, "title": "乙类大额",
                       "filter": { "op": "and", "args": [
                           { "op": "cmp", "field_id": "f3", "operator": "eq", "value": "b" },
                           { "op": "cmp", "field_id": "f5", "operator": "gt", "value": 50000 } ] },
                       "sorts": [{ "field_id": "f1", "direction": "desc" }],
                       "fields": [{ "field_id": "title" }, { "field_id": "f1" }, { "field_id": "f3" }, { "field_id": "f5" }] } },
        { "op": "entity.create", "entity_id": "view-group", "type_id": "buckyos.cell", "parent_id": "perf", "order_key": "b",
          "placement": { "x": 900, "y": 0, "w": 800, "h": 600 },
          "payload": { "source_ref": { "entity_id": "big" }, "view": { "type": "table" }, "title": "按状态分组",
                       "sorts": [{ "field_id": "f2", "direction": "asc" }], "group": { "field_id": "f3" } } },
    ]));

    // a long document: headings every 100 paragraphs, multi-byte text, a token only in the last paragraph
    let mut blocks = Vec::with_capacity(PARAGRAPHS + PARAGRAPHS / 100 + 1);
    let mut expected = String::new();
    for i in 0..PARAGRAPHS {
        if i % 100 == 0 {
            let h = format!("第 {} 章", i / 100 + 1);
            blocks.push(json!({ "type": "heading", "attrs": { "level": 2, "block_id": format!("h{i:05}") }, "content": [{ "type": "text", "text": h }] }));
        }
        let tail = if i == PARAGRAPHS - 1 { " 文末唯一词" } else { "" };
        let text = format!("第 {i} 段。{}{tail}", "这是一段用于性能验收的正文内容，包含中文、English words 与数字 12345。".repeat(2));
        blocks.push(json!({ "type": "paragraph", "attrs": { "block_id": format!("p{i:05}") }, "content": [{ "type": "text", "text": text }] }));
    }
    let nblocks = blocks.len();
    for b in &blocks {
        expected.push_str(b["content"][0]["text"].as_str().unwrap());
    }
    ok(ws, &alice(), json!([{ "op": "entity.create", "entity_id": "longdoc", "type_id": "buckyos.richtext", "parent_id": "data", "order_key": "r", "name": "长文档",
        "payload": { "content": { "type": "doc", "content": blocks } } }]));
    (expected, format!("{nblocks} blocks"))
}

/// The display columns of the large table, quoted (`"标题", "金额1", …`).
fn display_columns() -> String {
    let mut names = vec!["\"标题\"".to_string()];
    for i in 1..FIELDS {
        let kind = ["数值", "金额", "日期", "状态", "文本"][i % 5];
        names.push(format!("\"{kind}{i}\""));
    }
    names.join(", ")
}

fn q(ws: &Workspace, params: Value) -> Result<Value, aiworkspace_core::WsError> {
    ws.query(&alice(), &params)
}

/// Every page of a body: (texts, per-page ms).
fn read_pages(ws: &Workspace, id: &str, max_bytes: usize) -> (Vec<String>, Vec<f64>) {
    let mut texts = Vec::new();
    let mut times = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut p = json!({ "entity_id": id, "as": "body", "max_bytes": max_bytes });
        if let Some(c) = &cursor {
            p["cursor"] = json!(c);
        }
        let t = Instant::now();
        let page = ws.read_as(&alice(), &p).unwrap();
        times.push(ms(t));
        texts.push(page["text"].as_str().unwrap().to_string());
        match page["next_cursor"].as_str() {
            Some(c) if page["complete"] == json!(false) => cursor = Some(c.to_string()),
            _ => break,
        }
    }
    (texts, times)
}

fn pct(times: &[f64]) -> (f64, f64, f64) {
    let mut s = times.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (s[s.len() / 2], s[(s.len() * 95 / 100).min(s.len() - 1)], s[s.len() - 1])
}

/// A text query followed to the end: (hits, calls, per-call ms, every non-final answer was incomplete).
fn text_all(ws: &Workspace, params: Value) -> (Vec<Value>, usize, Vec<f64>) {
    let mut hits = Vec::new();
    let mut times = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut p = params.clone();
        if let Some(c) = &cursor {
            p["cursor"] = json!(c);
        }
        let t = Instant::now();
        let r = q(ws, p).unwrap();
        times.push(ms(t));
        hits.extend(r["hits"].as_array().unwrap().iter().cloned());
        match r["next_cursor"].as_str() {
            Some(c) => {
                assert_eq!(r["coverage"]["complete"], false, "an answer with a cursor must not claim complete coverage");
                cursor = Some(c.to_string());
            }
            None => {
                assert_eq!(r["coverage"]["complete"], true);
                break;
            }
        }
    }
    let calls = times.len();
    (hits, calls, times)
}

#[test]
#[ignore]
fn core_api_acceptance() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "性能验收", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let mut r = Report(Vec::new());

    let t = Instant::now();
    let (doc_text, doc_shape) = load(&mut ws);
    println!("\n== fixture: {ROWS} records × {FIELDS} fields, {DEPT}-row table, long document ({doc_shape}, {} chars) ==", doc_text.chars().count());
    r.row("load", format!("{:.1} s", t.elapsed().as_secs_f64()));
    if let Ok(dir) = std::env::var("AIWS_PERF_DUMP") {
        // the loaded Workspace for looking at query plans by hand (doc.sqlite and its WAL)
        for f in ["doc.sqlite", "doc.sqlite-wal"] {
            let _ = std::fs::copy(ws.dir.join(f), std::path::Path::new(&dir).join(f));
        }
        let ddl = ws.read_ctx(|ctx| {
            let access = aiworkspace_core::access::Access::full("alice");
            Ok::<_, aiworkspace_core::WsError>([("big", "大表"), ("dept", "部门")].iter().map(|(id, name)| {
                aiworkspace_core::sqlview::logical_table_ddl(ctx, &access, &ctx.entity(id).unwrap().unwrap(), name).unwrap()
            }).collect::<Vec<_>>().join(";\n"))
        }).unwrap();
        std::fs::write(std::path::Path::new(&dir).join("views.sql"), ddl).unwrap();
        return;
    }
    let n = 5;

    // ---- SQL
    println!("\n-- SQL (p50 / max of {n} runs, ms)");
    let sql_runs: Vec<(&str, String, Value)> = vec![
        ("count(*)", r#"SELECT count(*) FROM "/data/大表""#.into(), json!(null)),
        ("filter + sort, first page of 100", r#"SELECT _id, "标题", "金额1" FROM "/data/大表" WHERE "状态3#raw" = 'b' AND "数值5" > 50000 ORDER BY ("金额1" IS NULL), "金额1" DESC, _id LIMIT 100"#.into(), json!(null)),
        ("filter + sort, page at offset 30 000", r#"SELECT _id, "标题", "金额1" FROM "/data/大表" WHERE "状态3#raw" = 'b' ORDER BY ("金额1" IS NULL), "金额1" DESC, _id LIMIT 100 OFFSET 30000"#.into(), json!(null)),
        ("group by select, sum decimal", r#"SELECT "状态3", count(*), sum("金额1") FROM "/data/大表" GROUP BY 1 ORDER BY 1"#.into(), json!(null)),
        ("LIKE over a text column", r#"SELECT count(*) FROM "/data/大表" WHERE "文本4" LIKE '%12345%'"#.into(), json!(null)),
        ("date range + order by date", r#"SELECT _id, "日期2" FROM "/data/大表" WHERE "日期2" BETWEEN '2026-03-01' AND '2026-03-31' ORDER BY "日期2", _id LIMIT 100"#.into(), json!(null)),
        ("join a 1000-row table (materialized CTE), group", r#"WITH d AS MATERIALIZED (SELECT "编号" AS code, "名称" AS name FROM "/data/部门") SELECT d.name, count(*) FROM "/data/大表" b JOIN d ON (b."数值5" % 1000) = d.code GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 10"#.into(), json!(null)),
        ("5000 rows × 20 display columns", format!(r#"SELECT _id, {} FROM "/data/大表" ORDER BY _id"#, display_columns()), json!(5000)),
    ];
    let mut sql_max = 0f64;
    // a join written plainly: the logical tables are views over JSON, so the join key is an expression
    let t = Instant::now();
    let plain = q(&ws, json!({ "sql": r#"SELECT d."名称", count(*) FROM "/data/大表" b JOIN "/data/部门" d ON (b."数值5" % 1000) = d."编号" GROUP BY 1 ORDER BY 2 DESC, 1 LIMIT 10"# }));
    r.row("sql join written plainly", format!("{:>8.1}   {}", ms(t), plain.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| format!("{} {}", e.code.as_str(), e.detail))));
    if let Err(e) = &plain {
        assert!(e.detail.contains("AS MATERIALIZED"), "a slow join says how to write it: {e:?}");
    }
    for (what, sql, max_rows) in &sql_runs {
        let mut p = json!({ "sql": sql });
        if !max_rows.is_null() {
            p["max_rows"] = max_rows.clone();
        }
        let (p50, max, out) = bench(n, || q(&ws, p.clone()).unwrap());
        sql_max = sql_max.max(max);
        r.at_most(&format!("sql {what}"), max, SQL_CALL_MS);
        r.row(&format!("sql {what}"), format!("{p50:>8.1} / {max:>8.1}   rows {}", out["rows"].as_array().unwrap().len()));
    }

    // what a Desktop without the replica runs per render of a saved view (tableQuery.ts): page, count, groups
    let (cell_open, cell_group) = ws.read_ctx(|ctx| Ok::<_, aiworkspace_core::WsError>((ctx.entity("view-open")?.unwrap(), ctx.entity("view-group")?.unwrap()))).unwrap();
    let fields = ws.read_ctx(|ctx| aiworkspace_core::filter::live_fields(ctx, "big")).unwrap();
    for (what, cell) in [("view (filter, sort, 4 fields)", &cell_open), ("view (sort, group)", &cell_group)] {
        let c = compile_view(&ident("t"), &fields, &ViewDef::of_cell(cell), Projection::Records).unwrap();
        let (p50, max, page) = bench(n, || {
            let page = q(&ws, json!({ "sql": format!("{} LIMIT ? OFFSET ?", c.sql), "params": [100, 0], "tables": { "t": "big" }, "max_rows": 100 })).unwrap();
            let count = q(&ws, json!({ "sql": c.count_sql, "tables": { "t": "big" } })).unwrap();
            if let Some(g) = &c.group_sql {
                q(&ws, json!({ "sql": g, "tables": { "t": "big" }, "max_rows": 5000 })).unwrap();
            }
            (page, count)
        });
        sql_max = sql_max.max(max);
        r.at_most(&format!("Desktop render of a {what}"), max, VIEW_RENDER_MS);
        r.row(&format!("Desktop render of a {what}: page+count(+groups)"), format!("{p50:>8.1} / {max:>8.1}   total {}", page.1["rows"][0][0]));
        // the evaluator (a replica tab) gives the same first page
        let ids: Vec<String> = page.0["rows"].as_array().unwrap().iter().map(|row| serde_json::from_str::<Value>(row[0].as_str().unwrap()).unwrap()["record_id"].as_str().unwrap().to_string()).collect();
        let def = ViewDef::of_cell(cell);
        let spec = aiworkspace_core::filter::QuerySpec { limit: 100, best_effort: true, filter: def.filter.clone(), sorts: def.sorts.clone(), group: def.group.clone(), fields: def.fields.clone(), ..Default::default() };
        let t = Instant::now();
        let want = ws.read_ctx(|ctx| aiworkspace_core::filter::run_query(ctx, &ctx.entity("big")?.unwrap(), &spec)).unwrap();
        r.row("  evaluator, same view, first page", format!("{:>8.1}", ms(t)));
        let want: Vec<String> = want["rows"].as_array().unwrap().iter().map(|x| x["record_id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids, want, "{what}: SQL and the evaluator give different rows");
    }

    // budgets: rows truncate, bytes and time refuse with a structured error — never a silent cut
    let rows = q(&ws, json!({ "sql": r#"SELECT _id FROM "/data/大表""#, "max_rows": 5000 })).unwrap();
    assert_eq!((rows["rows"].as_array().unwrap().len(), rows["truncated"].clone()), (5000, json!(true)));
    let t = Instant::now();
    let bytes = q(&ws, json!({ "sql": r#"SELECT _record, _record FROM "/data/大表""#, "max_rows": 5000 })).unwrap_err();
    r.row("bytes budget (5000 × 2 records > 4 MiB) refused after", format!("{:>8.1}   {} {}", ms(t), bytes.code.as_str(), bytes.data.as_ref().map(|d| d["limit"].to_string()).unwrap_or_default()));
    assert_eq!(bytes.data.as_ref().map(|d| d["limit"].clone()), Some(json!("bytes")), "{bytes:?}");
    let t = Instant::now();
    let slow = q(&ws, json!({ "sql": r#"SELECT count(*) FROM "/data/大表" a JOIN "/data/大表" b ON a."文本4" < b."文本4""# })).unwrap_err();
    let stopped = ms(t);
    r.row("time budget (self-join) refused after", format!("{stopped:>8.1}   {} {}", slow.code.as_str(), slow.data.as_ref().map(|d| d["limit"].to_string()).unwrap_or_default()));
    assert_eq!(slow.data.as_ref().map(|d| d["limit"].clone()), Some(json!("time")), "{slow:?}");
    r.at_most("time budget refusal", stopped, 2000.0 + TIME_BUDGET_SLACK_MS);

    // ---- body
    println!("\n-- body (doc.read as body)");
    for max in [32 * 1024, 256 * 1024] {
        let t = Instant::now();
        let (pages, times) = read_pages(&ws, "big", max);
        let total = ms(t);
        let (p50, p95, pmax) = pct(&times);
        let csv = pages.concat();
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), ROWS + 1, "a header and one line per record");
        assert!(lines[0].starts_with("_id,标题,金额1"), "{}", lines[0]);
        assert_eq!(lines.iter().filter(|l| l.starts_with("_id,")).count(), 1, "the header only once");
        for (i, l) in lines[1..].iter().enumerate() {
            assert!(l.starts_with(&format!("r{i:06},")), "record {i} missing or out of order: {l}");
        }
        r.at_most(&format!("table CSV {} KiB page p95", max / 1024), p95, if max == 32 * 1024 { TABLE_PAGE_32K_MS } else { TABLE_PAGE_256K_MS });
        r.at_most(&format!("table CSV {} KiB slowest page against the median", max / 1024), pmax, 3.0 * p50 + 5.0);
        r.at_most(&format!("table CSV {} KiB, all pages", max / 1024), total, TABLE_ALL_MS);
        r.row(&format!("table CSV, {} KiB pages: {} pages, {:.1} MiB", max / 1024, pages.len(), csv.len() as f64 / 1048576.0),
              format!("page p50 {p50:.1} p95 {p95:.1} max {pmax:.1}; first {:.1} last {:.1}; all {:.0} ms", times[0], times[times.len() - 1], total));
    }
    for max in [32 * 1024, 256 * 1024] {
        let t = Instant::now();
        let (pages, times) = read_pages(&ws, "longdoc", max);
        let total = ms(t);
        let (p50, p95, pmax) = pct(&times);
        let text = pages.concat();
        let got: String = text.split('\n').filter(|l| !l.is_empty()).collect::<Vec<_>>().concat();
        let got = got.replace("## ", "");
        assert_eq!(got.chars().count(), doc_text.chars().count(), "the pages lose or repeat characters");
        assert!(text.trim_end().ends_with("文末唯一词"), "the last paragraph");
        r.at_most(&format!("long document {} KiB slowest page", max / 1024), pmax, DOC_PAGE_MS);
        r.row(&format!("long document, {} KiB pages: {} pages, {:.1} MiB", max / 1024, pages.len(), text.len() as f64 / 1048576.0),
              format!("page p50 {p50:.1} p95 {p95:.1} max {pmax:.1}; first {:.1} last {:.1}; all {:.0} ms", times[0], times[times.len() - 1], total));
    }
    let (p50, max, read) = bench(n, || ws.read(&alice(), "big", None).unwrap());
    r.row("doc.read of the 100 000-row table (native content)", format!("{p50:>8.1} / {max:>8.1}   {} KiB", read.to_string().len() / 1024));
    r.at_most("doc.read of the table", max, TABLE_READ_MS);
    assert_eq!(read["content"]["record_count"], json!(ROWS));
    let (p50, max, _) = bench(n, || ws.read_as(&alice(), &json!({ "entity_id": "big", "as": "meta" })).unwrap());
    r.row("meta of the 100 000-row table", format!("{p50:>8.1} / {max:>8.1}"));
    r.at_most("meta of the table", max, META_MS);
    let (p50, max, _) = bench(n, || ws.read_as(&alice(), &json!({ "entity_id": "view-open", "as": "meta" })).unwrap());
    r.row("meta of a view Block (with view_sql)", format!("{p50:>8.1} / {max:>8.1}"));
    r.at_most("meta of a view Block", max, META_MS);

    // ---- text query
    println!("\n-- text query (doc.query text), followed to the end");
    let cases = [
        ("miss over the table", json!({ "text": "不存在的词", "at": "/data/大表" }), 0usize),
        ("hit only in the last record", json!({ "text": "末尾唯一词", "at": "/data/大表" }), 1),
        ("miss over the whole Workspace", json!({ "text": "不存在的词" }), 0),
        ("hit only at the end of the long document", json!({ "text": "文末唯一词", "at": "/data/长文档" }), 1),
    ];
    let mut text_call_max = 0f64;
    for (what, p, want) in cases {
        let t = Instant::now();
        let (hits, calls, times) = text_all(&ws, p);
        let total = ms(t);
        let (_, _, cmax) = pct(&times);
        text_call_max = text_call_max.max(cmax);
        assert_eq!(hits.len(), want, "{what}: {hits:?}");
        r.at_most(&format!("text {what}, slowest call"), cmax, TEXT_CALL_MS);
        r.at_most(&format!("text {what}, to the end"), total, TEXT_ALL_MS);
        r.row(&format!("text {what}"), format!("{calls} calls, per call max {cmax:.1}, all {total:.0} ms"));
    }
    let t = Instant::now();
    let common = q(&ws, json!({ "text": "文本内容", "at": "/data/大表", "limit": 20 })).unwrap();
    r.at_most("text common word, first answer", ms(t), TEXT_CALL_MS);
    r.row("text common word, first answer", format!("{:>8.1}   hits {} complete {}", ms(t), common["hits"].as_array().unwrap().len(), common["coverage"]["complete"]));
    let (p50, max, found) = bench(n, || ws.find(&alice(), &json!({ "name": "大表" })).unwrap());
    r.row("doc.find by name", format!("{p50:>8.1} / {max:>8.1}   items {}", found["items"].as_array().map_or(0, Vec::len)));
    r.at_most("doc.find by name", max, META_MS);

    println!("\nSQL slowest call {sql_max:.1} ms; text query slowest call {text_call_max:.1} ms");
    assert!(r.0.is_empty(), "pass conditions not met:\n{}", r.0.join("\n"));
}
