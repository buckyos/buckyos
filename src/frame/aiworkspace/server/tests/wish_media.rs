//! A macro's generated pictures (`aiws.ai.image`): the executor imports what the AI service made as a staged asset of
//! the Workspace, the macro holds only its object id, and `result.file` writes it as an image result — applied as an
//! asset with its size, produced again from the cache on a later run, and never an object the Workspace does not have.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use wishkit::*;

const PROGRAM: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: {}, ai: { image: { max_calls: 2 } } })
  const poster = await aiws.ai.image({ prompt: '雨夜快递员的电影海报，赛博朋克水彩', aspect: '3:4' })
  if (poster.kind !== 'image' || !poster.object_id.startsWith('cyfile:') || poster.width !== 64 || poster.height !== 48) throw new Error(JSON.stringify(poster))
  aiws.result.file('poster', poster, { title: '海报' })
}
"#;

const FOREIGN: &str = r#"
export default async function main(aiws) {
  aiws.declare({ inputs: {} })
  aiws.result.file('poster', { object_id: 'cyfile:' + 'ab'.repeat(32), media_type: 'image/png' })
}
"#;

fn analysis() -> Script {
    Box::new(|t: &Turn| match t.results.len() {
        0 => call("submit_analysis", json!({ "analysis": {
            "schema_version": "wish.analysis.v3", "status": "ready", "inputs": [],
            "context_prompt": "宏调用 aiws.ai.image 生成一张竖版电影海报。",
            "output_contract": { "results": [{ "name": "poster", "type": "image", "title": "海报" }] },
            "checks": [], "blockers": [], "warnings": [] } })),
        _ => text("分析完成。"),
    })
}

fn coding(source: &'static str) -> Script {
    Box::new(move |t: &Turn| match t.results.len() {
        0 => call("run_program", json!({ "source": source })),
        1 => call("finish", json!({ "summary": "完成" })),
        _ => text("已完成。"),
    })
}

#[test]
fn generated_pictures_are_staged_assets_the_macro_writes_by_id() {
    run_test(|| async move {
        let m = model();
        *m.analyze.lock().unwrap() = analysis();
        *m.execute.lock().unwrap() = coding(PROGRAM);
        let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
        srv.ws = srv.rpc("ws.create", json!({ "title": "短片" })).await["workspace_id"].as_str().unwrap().to_string();
        srv.commit(json!([
            { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "s",
              "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "短片" } },
            { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "s", "name": "短片",
              "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "短片", "content_folder_id": "sf-content" } },
            { "op": "entity.create", "entity_id": "wish-poster", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "w", "name": "海报",
              "payload": { "title": "海报", "prompt": "画一张电影海报", "executor": "xllm", "output_mode": "overwrite",
                           "output": { "container_id": "sf-content", "surface_id": "sf", "name": "海报结果" } } },
        ])).await;
        let a = srv.run("wish.xllm@1", json!({ "wish_id": "wish-poster", "stage": "analyze" })).await;
        assert_eq!(a["state"], "waiting_confirmation", "{a}");
        srv.apply(&a, json!({})).await;

        // coding: the picture is generated once and is the candidate's image result, with its size and a file name
        let c = srv.run("wish.xllm@1", json!({ "wish_id": "wish-poster", "stage": "code" })).await;
        assert_eq!(c["state"], "waiting_confirmation", "{c}");
        assert_eq!(m.image_calls.load(Ordering::SeqCst), 1);
        assert_eq!(m.budget.lock().unwrap()["image_model"], "image.txt2img", "the stage budget names the media models");
        let poster = c["candidate"]["results"].as_array().unwrap().iter().find(|r| r["name"] == "poster").unwrap().clone();
        assert_eq!(poster["type"], "image", "{poster}");
        let id = poster["file"]["object_id"].as_str().unwrap().to_string();
        assert!(id.starts_with("cyfile:"), "{poster}");
        assert_eq!(poster["file"]["image"], json!({ "width": 64, "height": 48 }), "{poster}");
        assert_eq!(poster["file"]["file_name"], "poster.png");
        assert_eq!(c["candidate"]["program"]["manifest"]["ai"]["image"]["max_calls"], 2);
        srv.apply(&c, json!({})).await;
        let w = srv.wish_of("wish-poster").await;
        let asset = w["last_run"]["result_bindings"]["results"]["poster"]["entity_id"].as_str().unwrap().to_string();
        let read = srv.rpc("doc.read", json!({ "entity_id": asset })).await;
        assert_eq!(read["content"]["payload"]["object_id"], json!(id), "{read}");
        assert_eq!(read["content"]["payload"]["image"], json!({ "width": 64, "height": 48 }));
        assert!(srv.asset_bytes(&srv.ws, &id).await.starts_with(b"\x89PNG"));

        // running the stored macro: the same request is answered from the cache — the same picture, nothing generated
        let r = srv.run("wish.xllm@1", json!({ "wish_id": "wish-poster", "stage": "run" })).await;
        assert_eq!(r["state"], "waiting_confirmation", "{r}");
        assert_eq!(m.image_calls.load(Ordering::SeqCst), 1);
        let again: Vec<Value> = r["candidate"]["results"].as_array().unwrap().clone();
        assert_eq!(again[0]["file"]["object_id"], json!(id));

        // a macro cannot write an object of the store the Workspace does not have
        *m.execute.lock().unwrap() = coding(FOREIGN);
        let f = srv.run("wish.xllm@1", json!({ "wish_id": "wish-poster", "stage": "code" })).await;
        assert_eq!(f["state"], "failed", "{f}");
        assert!(f["error"]["detail"].as_str().unwrap_or("").contains("is not a file of this Workspace"), "{f}");
    });
}
