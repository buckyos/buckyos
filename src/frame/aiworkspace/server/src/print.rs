//! `print.package` (打印 TODO T7): the print package of one canvas, its rows taken in one go under the Workspace's
//! writer, zipped and handed to the caller (NDM holdings of the caller's own App session), so that the caller's own
//! `media.page` run may read it.

use crate::{blocking, s, AppState};
use aiworkspace_core::WsResult;
use aiworkspace_store::print::{renderer_files, PrintRequest, RENDERER_DIR};
use aiworkspace_store::Caller;
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn package(state: &Arc<AppState>, p: &Value, caller: &Caller) -> WsResult<Value> {
    let ws_id = s(p, "workspace_id")?.to_string();
    let req = PrintRequest::parse(p)?;
    let dir = state.limits.web_dir.as_ref().map(|d| d.join(RENDERER_DIR));
    let (st, caller) = (state.clone(), caller.clone());
    blocking(move || {
        let renderer = renderer_files(dir.as_deref())?;
        // the rows are taken under the writer; zipping and storing need it no more
        let (contents, objects) = st.with_ws(&ws_id, |ws| {
            ws.check_quota(0, false)?;
            Ok((ws.print_contents(&caller, &req)?, ws.objects.clone()))
        })?;
        let built = contents.store(&objects, &renderer)?;
        // a session without an App (the standalone test identities) has no NDM holdings to receive it
        match caller.app_id.as_deref() {
            Some(app) => st.svc.objects.grant_holdings(&caller.principal, app, std::slice::from_ref(&built.obj_id))?,
            None => log::info!("print package {} not handed to {}: the session names no app", built.obj_id, caller.principal),
        }
        Ok(json!({ "ok": true, "obj_id": built.obj_id, "size": built.size, "job": built.job, "issues": built.issues }))
    })
    .await
}
