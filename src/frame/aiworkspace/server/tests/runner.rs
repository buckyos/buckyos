use aiworkspace_server::wish::{runner, AIWS_JS, RUN_JS};
use axum::{http::HeaderMap, routing::{get, post}, Json, Router};
use serde_json::{json, Value};

#[tokio::test]
async fn program_reads_snapshot_writes_results_and_calls_network_on_rerun() {
    let deno = runner::find_deno("").expect("deno is needed for runner tests");
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("许愿格 space #%+");
    for path in ["lib", "program", "context/entities/rules"] {
        std::fs::create_dir_all(work.join(path)).unwrap();
    }
    std::fs::write(work.join("lib/aiws.js"), AIWS_JS).unwrap();
    std::fs::write(work.join("lib/run.js"), RUN_JS).unwrap();
    std::fs::write(work.join("context/inputs.json"), json!({
        "rules": { "entity_id": "rules", "type_id": "buckyos.richtext", "dir": "context/entities/rules" }
    }).to_string()).unwrap();
    std::fs::write(work.join("context/entities/rules/content.md"), "快照内容 #%+").unwrap();
    std::fs::write(work.join("request.json"), json!({ "currency": "CNY" }).to_string()).unwrap();
    std::fs::write(work.join("program/main.js"), r#"
export default async function main(aiws) {
  const snapshot = aiws.input('rules').markdown()
  const labels = await aiws.llm.map([snapshot], '返回输入', { type: 'string' })
  const host = Deno.env.get('AIWS_HOST')
  const external = await (await fetch(`${host}/external`)).text()
  aiws.result.text('note', labels[0])
  aiws.facts({ currency: aiws.request.currency, external })
  try {
    Deno.writeTextFileSync('program/forbidden.txt', 'x')
    throw new Error('write outside output was allowed')
  } catch (e) {
    if (!(e instanceof Deno.errors.NotCapable)) throw e
  }
}
"#).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = Router::new()
        .route("/llm_map", post(|headers: HeaderMap, Json(body): Json<Value>| async move {
            assert_eq!(headers["x-aiws-token"], "runner-token");
            assert_eq!(body["items"], json!(["快照内容 #%+"]));
            Json(json!({ "results": [{ "value": "快照内容 #%+" }], "calls": 1, "cached": 0 }))
        }))
        .route("/external", get(|| async { "network-ok" }));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for _ in 0..2 {
        let output = runner::run_in(
            &deno, &work,
            vec![("AIWS_HOST".into(), base.clone()), ("AIWS_TOKEN".into(), "runner-token".into())],
            30, 128, None,
        ).await.unwrap();
        assert!(!output.timed_out, "{}", output.stderr);
        assert_eq!(output.code, Some(0), "{}", output.stderr);
        let results = output.results.expect("results.json must be written");
        assert_eq!(results["error"], Value::Null);
        assert_eq!(results["results"][0]["markdown"], "快照内容 #%+");
        assert_eq!(results["facts"], json!({ "currency": "CNY", "external": "network-ok" }));
        assert_eq!(results["llm_map"]["items"], 1);
        assert!(!work.join("program/forbidden.txt").exists());
    }
    server.abort();
}
