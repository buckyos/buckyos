use crate::{blocking, AppState};
use aiworkspace_core::{value::parse_datetime_ms, Code, WsError, WsResult};
use aiworkspace_store::Caller;
use serde_json::{json, Value};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

pub(crate) async fn grant(token: &str, did: &str) -> WsResult<Value> {
    let rt = buckyos_api::get_buckyos_api_runtime().map_err(|_| unavailable())?;
    let url = format!(
        "http://{}:{}/kapi/control-panel",
        rt.get_local_gateway_host(),
        rt.node_gateway_port
    );
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| unavailable())?;
    let response = client
        .post(url)
        .json(&json!({"method":"owner.sharing.check","params":{"did":did},"sys":[0,token]}))
        .send()
        .await
        .map_err(|_| unavailable())?;
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(WsError::denied("availability has not been shared"));
    }
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let body: Value = response.json().await.map_err(|_| unavailable())?;
    grant_reply(&body)
}

fn grant_reply(body: &Value) -> WsResult<Value> {
    if let Some(error) = body.get("error").filter(|error| !error.is_null()) {
        let msg = [
            body["code"].as_str(),
            error["code"].as_str(),
            error.as_str(),
            error["message"].as_str(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
        return Err(
            if [
                "permission",
                "denied",
                "forbidden",
                "session",
                "invalid token",
            ]
            .iter()
            .any(|term| msg.contains(term))
            {
                WsError::denied("availability has not been shared")
            } else {
                unavailable()
            },
        );
    }
    body.get("result")
        .filter(|r| r["owner_user_id"].is_string() && r["mode"].is_string())
        .cloned()
        .ok_or_else(unavailable)
}

fn unavailable() -> WsError {
    WsError::new(
        Code::DependencyUnavailable,
        "the availability source is unavailable",
    )
}

fn datetime(p: &Value, key: &str) -> WsResult<i64> {
    p[key]
        .as_str()
        .filter(|s| s.is_ascii())
        .and_then(parse_datetime_ms)
        .ok_or_else(|| WsError::invalid_op(format!("{key} requires a timestamp with an offset")))
}

const MAX_ROWS: usize = 10000;
const MAX_BYTES: usize = 8 * 1024 * 1024;
fn rows(state: &AppState, owner: &str) -> WsResult<(Vec<Value>, bool, Option<String>)> {
    let caller = Caller::user(owner);
    let lookup = state
        .svc
        .lookup_workspace(&caller, "schedule")
        .map_err(|_| unavailable())?;
    if lookup["found"] == false {
        return Ok((Vec::new(), false, None));
    }
    let ws_id = lookup["workspace_id"].as_str().ok_or_else(unavailable)?;
    let tz = state
        .svc
        .users
        .get_prefs(&caller)
        .map_err(|_| unavailable())?["prefs"]["timezone"]
        .as_str()
        .map(str::to_owned);
    state.with_ws(ws_id, |ws| {
        if ws.owner()?.as_deref() != Some(owner) {
            return Err(unavailable());
        }
        let table = match ws.read(&caller, "calendar", None) {
            Ok(table) => table,
            Err(e) if matches!(e.code, Code::NotFound | Code::TargetDeleted) => {
                return Ok((Vec::new(), false, tz));
            }
            Err(_) => return Err(unavailable()),
        };
        if table["schema_id"] != "buckyos.calendar@1" || table["type_id"] != "buckyos.table-source"
        {
            return Err(unavailable());
        }
        const PAGE_SIZE: usize = 128;
        let mut records = Vec::new();
        let mut bytes = 0;
        loop {
            let query = json!({
                "sql": r#"SELECT "_record" FROM t ORDER BY "_id" LIMIT ? OFFSET ?"#,
                "params": [PAGE_SIZE, records.len()],
                "tables": {"t":"calendar"},
                "max_rows": PAGE_SIZE,
            });
            let page = ws.query(&caller, &query).map_err(|_| unavailable())?;
            let batch = page["rows"].as_array().ok_or_else(unavailable)?;
            for row in batch {
                let raw = row[0].as_str().ok_or_else(unavailable)?;
                bytes += raw.len();
                if records.len() == MAX_ROWS || bytes > MAX_BYTES {
                    return Ok((records, true, tz));
                }
                records.push(serde_json::from_str(raw).map_err(|_| unavailable())?);
            }
            if batch.len() < PAGE_SIZE {
                return Ok((records, false, tz));
            }
        }
    })
}

async fn project(request: Value) -> WsResult<Value> {
    let base = buckyos_kit::get_buckyos_root_dir().join("libexec/buckyos-tool");
    let mut child =
        Command::new(
            base.join("runtime")
                .join(if cfg!(windows) { "deno.exe" } else { "deno" }),
        )
        .args([
            "run",
            "--no-config",
            "--no-prompt",
            "--cached-only",
            "--no-npm",
            "--no-lock",
        ])
        .arg(base.join("dist/owner-extensions/scripts/free_busy.mjs"))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| unavailable())?;
    let mut stdin = child.stdin.take().ok_or_else(unavailable)?;
    let output = tokio::time::timeout(Duration::from_secs(10), async move {
        stdin.write_all(request.to_string().as_bytes()).await?;
        drop(stdin);
        child.wait_with_output().await
    })
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    if !output.status.success() || output.stdout.len() > 2 * 1024 * 1024 {
        return Err(unavailable());
    }
    let raw: Value = serde_json::from_slice(&output.stdout).map_err(|_| unavailable())?;
    let coverage = raw["coverage"]
        .as_str()
        .filter(|v| matches!(*v, "complete" | "partial"))
        .ok_or_else(unavailable)?;
    let mut intervals = Vec::new();
    for v in raw["intervals"].as_array().ok_or_else(unavailable)? {
        datetime(v, "start")?;
        datetime(v, "end")?;
        intervals.push(json!({"start":v["start"],"end":v["end"]}));
    }
    Ok(json!({"intervals":intervals,"coverage":coverage}))
}

pub(crate) async fn handle(
    state: &Arc<AppState>,
    method: &str,
    p: &Value,
    token: &str,
) -> WsResult<Value> {
    let subject = p["did"]
        .as_str()
        .ok_or_else(|| WsError::invalid_op("subject DID required"))?;
    let grant = state.auth.availability_grant(token, subject).await?;
    if method == "availability.describe" {
        return Ok(
            json!({"availability_ref":buckyos_api::obj_service_url("aiworkspace",&["people",subject,"availability"])}),
        );
    }
    let from = datetime(p, "from")?;
    let to = datetime(p, "to")?;
    if to <= from || to - from > 366 * 86400000i64 {
        return Err(WsError::invalid_op(
            "choose a positive window of up to one year",
        ));
    }
    match grant["mode"].as_str() {
        Some("default") => {}
        Some("disabled") => {
            return Err(WsError::sub(
                Code::DependencyUnavailable,
                "RESOURCE_DISABLED",
                "the shared calendar is disabled",
            ))
        }
        _ => {
            return Err(WsError::sub(
                Code::DependencyUnavailable,
                "AVAILABILITY_UNAVAILABLE",
                "the current source does not provide a busy-time projection",
            ))
        }
    }
    let owner = grant["owner_user_id"]
        .as_str()
        .ok_or_else(unavailable)?
        .to_owned();
    let st = state.clone();
    let (rows, truncated, tz) = blocking(move || rows(&st, &owner)).await?;
    let result = if rows.is_empty() && !truncated {
        json!({"intervals":[],"coverage":"complete"})
    } else {
        project(json!({"from":p["from"],"to":p["to"],"rows":rows,"truncated":truncated,"tz":tz}))
            .await?
    };
    if state.auth.availability_grant(token, subject).await? != grant {
        return Err(WsError::denied("availability sharing or source changed"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_revoked_grants_from_service_errors() {
        for reply in [
            json!({"error":"no permission"}),
            json!({"error":{"code":"PERMISSION_DENIED","message":"private"}}),
            json!({"error":"refused","code":"SESSION_EXPIRED"}),
        ] {
            assert_eq!(
                grant_reply(&reply).unwrap_err().code,
                Code::PermissionDenied
            );
        }
        let unavailable = json!({"error":"system-config failed"});
        assert_eq!(
            grant_reply(&unavailable).unwrap_err().code,
            Code::DependencyUnavailable
        );
        let valid = json!({"result":{"owner_user_id":"alice","mode":"default"}});
        assert_eq!(grant_reply(&valid).unwrap(), valid["result"]);
    }
}
