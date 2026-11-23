use crate::{
    adapters::{
        did_object::action_response_to_adapter_response, AdapterReadResponse, AdapterXCallResponse,
    },
    extensions::failure,
    owner_host, AgentDIDObjectError as Error, ReadInput, RouteMethod, RouteTrace, XCallInput,
};
use buckyos_api::object_extensions::{extension_file, LocalScriptObject};
use name_lib::ActionResponse;
use serde_json::{json, Value};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;

fn trace(method: RouteMethod) -> RouteTrace {
    RouteTrace {
        route_id: "ext-object".into(),
        adapter: "local_script".into(),
        method,
    }
}

pub(crate) fn read(
    object: LocalScriptObject,
    input: &ReadInput,
) -> Result<AdapterReadResponse, Error> {
    if input
        .options
        .get("properties")
        .and_then(Value::as_array)
        .is_some_and(|p| !p.is_empty())
    {
        return Err(failure(
            "UNSUPPORTED_PROPERTY",
            "local objects expose their profile",
        ));
    }
    let mut result = AdapterReadResponse::text(
        input.object.clone(),
        serde_json::to_string_pretty(&object.profile).unwrap(),
        trace(RouteMethod::Read),
    );
    result.meta.title = object.profile["title"].as_str().map(str::to_owned);
    result.meta.source = Some("local".into());
    result.meta.content_type = Some("application/did-object-profile+json".into());
    Ok(result)
}

async fn read_frame(
    reader: &mut (impl AsyncBufRead + Unpin),
    remaining: &mut usize,
) -> Result<Value, Error> {
    let mut frame = Vec::new();
    loop {
        let buf = reader.fill_buf().await?;
        if buf.is_empty() {
            return Err(failure(
                "INVALID_SCRIPT_RESPONSE",
                "script ended without a receipt",
            ));
        }
        let end = buf.iter().position(|b| *b == b'\n').map(|v| v + 1);
        let used = end.unwrap_or(buf.len());
        *remaining = remaining
            .checked_sub(used)
            .ok_or_else(|| failure("INVALID_SCRIPT_RESPONSE", "script exceeded output limit"))?;
        frame.extend_from_slice(&buf[..used]);
        reader.consume(used);
        if end.is_some() {
            break;
        }
    }
    serde_json::from_slice(&frame)
        .map_err(|_| failure("INVALID_SCRIPT_RESPONSE", "invalid script output"))
}

fn deno_binary(root: &Path) -> std::path::PathBuf {
    let exe = if cfg!(windows) { "deno.exe" } else { "deno" };
    let bundled = root.join("libexec/buckyos-tool/runtime").join(exe);
    if bundled.is_file() {
        bundled
    } else {
        root.join("bin/opendan").join(exe)
    }
}

pub(crate) async fn invoke(
    object: LocalScriptObject,
    input: &XCallInput,
    mut ctx: Value,
) -> Result<AdapterXCallResponse, Error> {
    ctx["invocation_id"] = json!(uuid::Uuid::new_v4().to_string());
    let Some(action) = object.actions.get(&input.action) else {
        return Err(failure(
            "UNSUPPORTED_ACTION",
            format!("undeclared action {}", input.action),
        ));
    };
    let spec = &object.profile["actions"][&input.action];
    let policy = &spec["x-buckyos:action"];
    let writing = !matches!(policy["effect"].as_str(), Some("read" | "pure"));
    if writing && ctx["read_only"].as_bool() != Some(false) {
        return Err(failure(
            "PERMISSION_DENIED",
            "write action requires a writable session",
        ));
    }
    if policy["confirm"] == "required" && input.confirm_token.as_deref().unwrap_or("").is_empty() {
        return Err(failure(
            "CONFIRMATION_REQUIRED",
            "this action requires confirmation",
        ));
    }
    if let Some(schema) = spec.get("input") {
        buckyos_api::validate_json_schema(schema, &input.params)
            .map_err(|e| failure("INVALID_PARAMS", e))?;
    }
    let script = extension_file(&object.root, &action.script)
        .map_err(|e| failure("EXTENSION_UNAVAILABLE", e))?;
    let script = url::Url::from_file_path(script)
        .map_err(|_| failure("EXTENSION_UNAVAILABLE", "invalid installed script"))?;
    let runner = format!(
        "data:text/javascript,{}",
        percent_encoding::utf8_percent_encode(
            include_str!("action_runner.js"),
            percent_encoding::NON_ALPHANUMERIC
        )
    );
    let mut child = Command::new(deno_binary(&buckyos_kit::get_buckyos_root_dir()))
        .env_clear()
        .current_dir(&object.root)
        .arg("run")
        .args([
            "--no-config",
            "--no-prompt",
            "--cached-only",
            "--no-npm",
            "--no-lock",
        ])
        .arg(format!("--allow-read={}", object.root.display()))
        .arg(runner)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| {
            failure(
                "EXTENSION_UNAVAILABLE",
                "cannot start the installed Deno runner",
            )
        })?;
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let request = json!({"method":input.action,"params":input.params,"idempotency_key":input.idempotency_key,"confirm_token":input.confirm_token,"trace_id":input.trace_id});
    let mut dispatched = false;
    let run = async {
        stdin
            .write_all(
                format!(
                    "{}\n",
                    json!({"script":script.as_str(),"request":request,"context":ctx})
                )
                .as_bytes(),
            )
            .await?;
        let mut remaining = OUTPUT_LIMIT;
        loop {
            let frame = read_frame(&mut stdout, &mut remaining).await?;
            if let Some(response) = frame.get("response") {
                let response: ActionResponse = serde_json::from_value(response.clone())
                    .map_err(|_| failure("INVALID_SCRIPT_RESPONSE", "invalid action response"))?;
                response
                    .validate()
                    .map_err(|_| failure("INVALID_SCRIPT_RESPONSE", "invalid action receipt"))?;
                if let (Some(result), Some(schema)) = (&response.result, spec.get("output")) {
                    buckyos_api::validate_json_schema(schema, result)
                        .map_err(|e| failure("INVALID_SCRIPT_RESPONSE", e))?;
                }
                return Ok(response);
            }
            let name = frame["method"]
                .as_str()
                .ok_or_else(|| failure("INVALID_SCRIPT_RESPONSE", "expected host call"))?;
            let id = frame["id"]
                .as_u64()
                .ok_or_else(|| failure("INVALID_SCRIPT_RESPONSE", "missing call id"))?;
            if !action.host_calls.iter().any(|call| call == name) {
                return Err(failure("HOST_CALL_DENIED", "undeclared host call"));
            }
            let host_writes = owner_host::writes(input, name);
            if host_writes && !writing {
                return Err(failure("HOST_CALL_DENIED", "a read action cannot write"));
            }
            dispatched |= host_writes;
            let reply = match owner_host::call(&ctx, input, name, frame["params"].clone()).await {
                Ok(result) => json!({"id":id,"result":result}),
                Err(e) => json!({"id":id,"error":e.detail()}),
            };
            stdin.write_all(format!("{reply}\n").as_bytes()).await?;
        }
    };
    let result = tokio::time::timeout(Duration::from_secs(60), run)
        .await
        .unwrap_or_else(|_| Err(failure("SCRIPT_TIMEOUT", "script timed out")));
    let _ = child.kill().await;
    let response = result.map_err(|e| {
        if dispatched {
            Error::Business {
                code: "WRITE_OUTCOME_UNKNOWN".into(),
                message: "script did not confirm the write".into(),
                detail: json!({"idempotency_key":input.idempotency_key,"trace_id":input.trace_id}),
            }
        } else {
            e
        }
    })?;
    Ok(action_response_to_adapter_response(
        &input.action,
        response,
        trace(RouteMethod::XCall),
    ))
}
