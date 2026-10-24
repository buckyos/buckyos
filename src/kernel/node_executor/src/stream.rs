//! One Command Stream: login, then requests until close (doc §7.2–7.4).

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use buckyos_api::{
    encode_executor_control, encode_executor_data, is_valid_executor_id,
    read_executor_frame, ExecutorClientMessage, ExecutorCloseReason, ExecutorClosing,
    ExecutorDataFrame, ExecutorDescribeOk, ExecutorDescribeRequest, ExecutorError, ExecutorErrorCode,
    ExecutorErrorReply, ExecutorFinished, ExecutorFrame, ExecutorLogin, ExecutorLoginOk, ExecutorOp,
    ExecutorOpReply, ExecutorOpSpec, ExecutorPosture, ExecutorQueryReply, ExecutorRuntimeInfo,
    ExecutorRuntimeTarget, ExecutorServerMessage, ExecutorStarted, ExecutorWriteRequest,
    EXECUTOR_MAX_DATA_BYTES, EXECUTOR_UPLOAD_DEADLINE_MS, NODE_EXECUTOR_PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

use crate::auth::{self, Grant};
use crate::ops::{CancelReason, OpContext, OpEvent};
use crate::spawn::{run_file_job, FileJobOutcome};
use crate::store::{op_key_hash, OpIdentity};
use crate::worker::WorkerJob;
use crate::{now_ms, ExecutorInner, StreamEntry};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);
const OUT_QUEUE: usize = 256;
const PER_STREAM_REQUESTS: usize = 8;

struct Session {
    id: u64,
    runtime_id: String,
    run_id: String,
    token_expires_at_ms: u64,
}

struct PendingUpload {
    request: ExecutorWriteRequest,
    path: PathBuf,
    file: std::fs::File,
    received: u64,
    hasher: Sha256,
    reserved: u64,
    deadline: tokio::time::Instant,
}

struct UploadState {
    inner: Arc<ExecutorInner>,
    pending: Option<PendingUpload>,
}

impl std::ops::Deref for UploadState {
    type Target = Option<PendingUpload>;
    fn deref(&self) -> &Self::Target { &self.pending }
}

impl std::ops::DerefMut for UploadState {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.pending }
}

impl Drop for UploadState {
    fn drop(&mut self) { abort_upload(&self.inner, &mut self.pending); }
}

struct StreamCleanup {
    inner: Arc<ExecutorInner>,
    id: Option<u64>,
    writer: tokio::task::AbortHandle,
    forwarder: Option<tokio::task::AbortHandle>,
}

impl Drop for StreamCleanup {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.inner.streams.lock().unwrap().remove(&id);
        }
        self.writer.abort();
        if let Some(forwarder) = &self.forwarder {
            forwarder.abort();
        }
    }
}

#[derive(Clone)]
struct Out {
    tx: mpsc::Sender<Vec<u8>>,
}

impl Out {
    async fn send(&self, msg: ExecutorServerMessage) {
        if let Ok(bytes) = encode_executor_control(&msg) {
            let _ = self.tx.send(bytes).await;
        }
    }

    fn try_send(&self, msg: ExecutorServerMessage) {
        if let Ok(bytes) = encode_executor_control(&msg) {
            let _ = self.tx.try_send(bytes);
        }
    }

    async fn error(&self, request_id: Option<String>, error: ExecutorError) {
        self.send(ExecutorServerMessage::Error(ExecutorErrorReply { request_id, error }))
            .await;
    }
}

fn e(code: ExecutorErrorCode, msg: impl Into<String>) -> ExecutorError {
    ExecutorError::new(code, msg)
}

pub(crate) async fn serve_stream<S>(inner: Arc<ExecutorInner>, stream: S)
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(OUT_QUEUE);
    let writer_task = tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if writer.write_all(&bytes).await.is_err() {
                break;
            }
            if rx.is_empty() && writer.flush().await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });
    let mut cleanup = StreamCleanup {
        inner: inner.clone(), id: None, writer: writer_task.abort_handle(), forwarder: None,
    };
    let out = Out { tx };

    let login = match tokio::time::timeout(LOGIN_TIMEOUT, read_executor_frame(&mut reader)).await {
        Ok(Ok(Some(ExecutorFrame::Control(payload)))) => {
            match serde_json::from_slice::<ExecutorClientMessage>(&payload) {
                Ok(ExecutorClientMessage::Login(login)) => Some(login),
                _ => None,
            }
        }
        _ => None,
    };
    let Some(login) = login else {
        out.error(None, e(ExecutorErrorCode::ProtocolError, "the first frame must be login"))
            .await;
        drop(out);
        let _ = writer_task.await;
        return;
    };
    let request_id = login.request_id.clone();
    let (session, close_rx) = match login_stream(&inner, &login).await {
        Ok(v) => v,
        Err(error) => {
            out.error(request_id, error).await;
            drop(out);
            let _ = writer_task.await;
            return;
        }
    };
    cleanup.id = Some(session.id);
    let config = inner.auth.current();
    let grant = current_grant(&inner, session.id);
    if let (Some(config), Some(grant)) = (config, grant) {
        out.send(ExecutorServerMessage::LoginOk(ExecutorLoginOk {
            request_id,
            posture: ExecutorPosture::Normal,
            principal: grant.actor.agent_did.clone(),
            app_instance_id: grant.actor.app_instance_id.clone(),
            caps: grant.caps.iter().copied().collect(),
            limits: config.limits.clone(),
            revision: config.revision.clone(),
            generation: config.generation,
            auth_valid_until_ms: config.auth_valid_until_ms,
            token_expires_at_ms: session.token_expires_at_ms,
            store_id: inner.store.store_id().to_string(),
            server_epoch: inner.server_epoch.clone(),
        }))
        .await;
    }

    let watched: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let forwarder = tokio::spawn(forward_events(
        inner.ops.subscribe(),
        watched.clone(),
        out.clone(),
    ));
    cleanup.forwarder = Some(forwarder.abort_handle());
    let session = Arc::new(session);
    let requests = Arc::new(tokio::sync::Semaphore::new(PER_STREAM_REQUESTS));
    let mut upload = UploadState { inner: inner.clone(), pending: None };
    let mut close_rx = close_rx;
    let token_deadline = tokio::time::Instant::now()
        + Duration::from_millis(session.token_expires_at_ms.saturating_sub(now_ms()));

    let closing: Option<ExecutorClosing> = loop {
        let upload_deadline = upload
            .as_ref()
            .map(|u| u.deadline)
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            frame = read_executor_frame(&mut reader) => {
                let frame = match frame {
                    Ok(Some(f)) => f,
                    Ok(None) => break None,
                    Err(err) => break Some(ExecutorClosing {
                        reason: ExecutorCloseReason::ProtocolError,
                        message: err.to_string(),
                    }),
                };
                match frame {
                    ExecutorFrame::Data(data) => {
                        if let Err(error) = on_data(&inner, &mut upload, data) {
                            abort_upload(&inner, &mut upload);
                            out.error(None, error).await;
                        }
                    }
                    ExecutorFrame::Control(payload) => {
                        let msg = match serde_json::from_slice::<ExecutorClientMessage>(&payload) {
                            Ok(m) => m,
                            Err(err) => {
                                let raw = serde_json::from_slice::<serde_json::Value>(&payload).ok();
                                let unsupported_ui = raw.as_ref()
                                    .and_then(|v| v.get("type"))
                                    .and_then(|v| v.as_str())
                                    .is_some_and(|kind| kind.starts_with("ui.") || kind.starts_with("ui_"));
                                if unsupported_ui {
                                    let request_id = raw.as_ref().and_then(|v| v.get("request_id"))
                                        .and_then(|v| v.as_str()).map(str::to_string);
                                    out.error(request_id, e(ExecutorErrorCode::Capability,
                                        "UI operations are unsupported on this host")).await;
                                } else {
                                    out.error(None, e(ExecutorErrorCode::ProtocolError, err.to_string())).await;
                                }
                                continue;
                            }
                        };
                        if let Some(closing) = check_session(&inner, &session) {
                            break Some(closing);
                        }
                        match msg {
                            ExecutorClientMessage::Login(_) => {
                                out.error(None, e(ExecutorErrorCode::ProtocolError, "already logged in")).await;
                            }
                            ExecutorClientMessage::Write(req) => {
                                if let Err(error) = begin_upload(&inner, &session, &mut upload, req.clone()) {
                                    out.error(Some(req.request_id), error).await;
                                }
                            }
                            ExecutorClientMessage::UploadEnd(end) => {
                                match upload.take() {
                                    Some(pending) if pending.request.op_id == end.op_id => {
                                        let request_id = pending.request.request_id.clone();
                                        let permit = match requests.clone().try_acquire_owned() {
                                            Ok(permit) => permit,
                                            Err(_) => {
                                                abort_upload(&inner, &mut Some(pending));
                                                out.error(Some(request_id), e(ExecutorErrorCode::Busy, "too many requests in flight on this stream")).await;
                                                continue;
                                            }
                                        };
                                        let inner = inner.clone();
                                        let session = session.clone();
                                        let watched = watched.clone();
                                        let out = out.clone();
                                        tokio::spawn(async move {
                                            let reply = finish_upload(&inner, &session, pending, &watched).await;
                                            send_op_reply(&out, request_id, reply, true).await;
                                            drop(permit);
                                        });
                                    }
                                    other => {
                                        *upload = other;
                                        out.error(Some(end.request_id), e(ExecutorErrorCode::NotFound, "no pending upload for this op_id")).await;
                                    }
                                }
                            }
                            ExecutorClientMessage::Cancel(r) if upload.as_ref().map(|u| u.request.op_id == r.op_id).unwrap_or(false) => {
                                abort_upload(&inner, &mut upload);
                                out.error(Some(r.request_id), e(ExecutorErrorCode::NotFound, "the upload was aborted; the write was never accepted")).await;
                            }
                            other => {
                                let permit = match requests.clone().try_acquire_owned() {
                                    Ok(p) => p,
                                    Err(_) => {
                                        out.error(other.request_id().map(String::from), e(ExecutorErrorCode::Busy, "too many requests in flight on this stream")).await;
                                        continue;
                                    }
                                };
                                let inner = inner.clone();
                                let session = session.clone();
                                let out = out.clone();
                                let watched = watched.clone();
                                tokio::spawn(async move {
                                    handle_request(&inner, &session, other, &out, &watched).await;
                                    drop(permit);
                                });
                            }
                        }
                    }
                }
            }
            changed = close_rx.changed() => {
                if changed.is_err() { break None; }
                let closing = close_rx.borrow().clone();
                if closing.is_some() { break closing; }
            }
            _ = tokio::time::sleep_until(token_deadline) => {
                break Some(ExecutorClosing {
                    reason: ExecutorCloseReason::TokenExpired,
                    message: "session token expired; log in again on a new stream".into(),
                });
            }
            _ = tokio::time::sleep_until(upload_deadline), if upload.is_some() => {
                let op_id = upload.as_ref().map(|u| u.request.op_id.clone());
                let request_id = upload.as_ref().map(|u| u.request.request_id.clone());
                abort_upload(&inner, &mut upload);
                out.error(request_id, e(ExecutorErrorCode::InvalidArgument, format!("upload of {} timed out", op_id.unwrap_or_default()))).await;
            }
        }
    };
    abort_upload(&inner, &mut upload);
    inner.streams.lock().unwrap().remove(&session.id);
    forwarder.abort();
    if let Some(closing) = closing {
        out.send(ExecutorServerMessage::Closing(closing)).await;
    }
    drop(out);
    let _ = writer_task.await;
}

async fn login_stream(
    inner: &Arc<ExecutorInner>,
    login: &ExecutorLogin,
) -> Result<(Session, watch::Receiver<Option<ExecutorClosing>>), ExecutorError> {
    if !inner.ops.is_accepting() {
        return Err(e(ExecutorErrorCode::Unavailable, "executor admission is paused"));
    }
    if login.v != NODE_EXECUTOR_PROTOCOL_VERSION {
        return Err(e(ExecutorErrorCode::ProtocolError, "unsupported protocol version"));
    }
    if !is_valid_executor_id(&login.client.runtime_id) || !is_valid_executor_id(&login.client.run_id) {
        return Err(e(ExecutorErrorCode::InvalidArgument, "invalid runtime_id or run_id"));
    }
    let config = inner.auth.current().ok_or_else(|| {
        e(ExecutorErrorCode::Unavailable, "the executor has no valid authorization snapshot")
    })?;
    let session = inner
        .verifier
        .verify(&login.token)
        .await
        .map_err(|_| e(ExecutorErrorCode::Unauthenticated, "invalid session token"))?;
    if session.expires_at_ms <= now_ms() {
        return Err(e(ExecutorErrorCode::Unauthenticated, "session token expired"));
    }
    let mut grant = auth::map_login(&session, &config, &login.profile, &inner.host_caps, inner.identity.mode)?;
    auth::apply_rbac(inner.rbac.as_ref(), &inner.node_id, inner.admission, &session, &mut grant).await;
    if !grant.caps.contains(&ExecutorOp::OpenStream) {
        return Err(e(ExecutorErrorCode::Forbidden, "RBAC denies executor.open_stream"));
    }
    let (close, close_rx) = watch::channel(None);
    let id = inner.next_stream.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    {
        let mut streams = inner.streams.lock().unwrap();
        if !inner.ops.is_accepting() {
            return Err(e(ExecutorErrorCode::Unavailable, "executor admission is paused"));
        }
        if streams.len() >= config.limits.max_streams as usize {
            return Err(e(ExecutorErrorCode::Busy, "max_streams reached"));
        }
        streams.insert(
            id,
            StreamEntry {
                session: session.clone(),
                grant,
                close,
            },
        );
    }
    Ok((
        Session {
            id,
            runtime_id: login.client.runtime_id.clone(),
            run_id: login.client.run_id.clone(),
            token_expires_at_ms: session.expires_at_ms,
        },
        close_rx,
    ))
}

fn current_grant(inner: &ExecutorInner, id: u64) -> Option<Grant> {
    inner.streams.lock().unwrap().get(&id).filter(|entry| entry.close.borrow().is_none()).map(|e| e.grant.clone())
}

fn check_session(inner: &ExecutorInner, session: &Session) -> Option<ExecutorClosing> {
    if session.token_expires_at_ms <= now_ms() {
        return Some(ExecutorClosing {
            reason: ExecutorCloseReason::TokenExpired,
            message: "session token expired; log in again on a new stream".into(),
        });
    }
    if inner.auth.current().is_none() {
        return Some(ExecutorClosing {
            reason: ExecutorCloseReason::AuthExpired,
            message: "authorization snapshot is not valid".into(),
        });
    }
    None
}

/// The grant and limits for one operation, or the error to return.
fn authorize(
    inner: &ExecutorInner,
    session: &Session,
    op: ExecutorOp,
    store_id: Option<&str>,
) -> Result<(Grant, OpContext), ExecutorError> {
    let admission_generation = inner.ops.admission_generation();
    if !inner.ops.is_accepting() {
        return Err(e(ExecutorErrorCode::Unavailable, "executor admission is paused"));
    }
    if !inner.host_caps.contains(&op) {
        return Err(e(
            ExecutorErrorCode::Capability,
            format!("this host cannot perform {}", op.as_str()),
        ));
    }
    let (config, lease_deadline_ms) = inner
        .auth
        .current_with_deadline()
        .ok_or_else(|| e(ExecutorErrorCode::Unavailable, "authorization snapshot is not valid"))?;
    let grant = current_grant(inner, session.id)
        .ok_or_else(|| e(ExecutorErrorCode::Forbidden, "stream is closing"))?;
    if config.profiles.get(&grant.profile_id) != Some(&grant.profile)
        || config.principal_bindings.get(&grant.actor.agent_did) != Some(&grant.binding)
    {
        return Err(e(ExecutorErrorCode::Forbidden, "authorization snapshot changed; reconnect"));
    }
    if !crate::CONTROL_OPS.contains(&op) && !inner.identity.mode.honors_run_as(&grant.profile) {
        return Err(e(ExecutorErrorCode::Capability, crate::RUN_AS_MISMATCH));
    }
    if !grant.caps.contains(&op) {
        return Err(e(
            ExecutorErrorCode::Forbidden,
            format!("{} is not granted", op.as_str()),
        ));
    }
    if let Some(store_id) = store_id {
        if store_id != inner.store.store_id() {
            return Err(e(
                ExecutorErrorCode::StoreMismatch,
                "store_id does not match this executor's ledger",
            ));
        }
    }
    let ctx = OpContext {
        profile: grant.profile.clone(),
        limits: config.limits.clone(),
        auth_generation: config.generation,
        admission_generation,
        auth_deadline_ms: auth::suspend_clock_ms().saturating_add(
            config.auth_valid_until_ms.min(session.token_expires_at_ms).saturating_sub(now_ms())).min(lease_deadline_ms),
    };
    Ok((grant, ctx))
}

fn identity(grant: &Grant, session: &Session, run_id: &str, call_id: &str, op_id: &str) -> Result<OpIdentity, ExecutorError> {
    if run_id != session.run_id {
        return Err(e(ExecutorErrorCode::InvalidArgument, "run_id differs from the login"));
    }
    for id in [call_id, op_id] {
        if !is_valid_executor_id(id) {
            return Err(e(ExecutorErrorCode::InvalidArgument, format!("invalid id `{id}`")));
        }
    }
    Ok(OpIdentity {
        actor: grant.actor.clone(),
        profile_id: grant.profile_id.clone(),
        runtime_id: session.runtime_id.clone(),
        run_id: run_id.to_string(),
        call_id: call_id.to_string(),
        op_id: op_id.to_string(),
    })
}

fn lookup_key(inner: &ExecutorInner, grant: &Grant, session: &Session, op_id: &str) -> String {
    op_key_hash(
        inner.store.store_id(),
        &grant.actor,
        &grant.profile_id,
        &session.runtime_id,
        op_id,
    )
}

async fn send_op_reply(
    out: &Out,
    request_id: String,
    reply: Result<(buckyos_api::ExecutorOpStatus, bool), ExecutorError>,
    accepted: bool,
) {
    match reply {
        Ok((op, duplicate)) => {
            let reply = ExecutorOpReply {
                request_id,
                op,
                duplicate,
            };
            out.send(if accepted {
                ExecutorServerMessage::Accepted(reply)
            } else {
                ExecutorServerMessage::Status(reply)
            })
            .await
        }
        Err(error) => out.error(Some(request_id), error).await,
    }
}

async fn submit(
    inner: &Arc<ExecutorInner>,
    session: &Session,
    op: ExecutorOp,
    store_id: &str,
    ids: (&str, &str, &str),
    spec: ExecutorOpSpec,
    upload: Option<(PathBuf, u64)>,
    watched: &Mutex<HashSet<String>>,
) -> Result<(buckyos_api::ExecutorOpStatus, bool), ExecutorError> {
    let release = |upload: &Option<(PathBuf, u64)>| {
        if let Some((path, bytes)) = upload {
            let _ = std::fs::remove_file(path);
            inner.store.release(*bytes);
        }
    };
    let (grant, ctx) = match authorize(inner, session, op, Some(store_id)) {
        Ok(v) => v,
        Err(err) => {
            release(&upload);
            return Err(err);
        }
    };
    if matches!(&spec, ExecutorOpSpec::Computer { request }
        if request.feedback().0 == buckyos_api::ComputerFeedback::After)
        && !grant.caps.contains(&ExecutorOp::ComputerObserve)
    {
        release(&upload);
        return Err(e(ExecutorErrorCode::Forbidden, "computer feedback requires computer_observe"));
    }
    let ident = match identity(&grant, session, ids.0, ids.1, ids.2) {
        Ok(v) => v,
        Err(err) => {
            release(&upload);
            return Err(err);
        }
    };
    let key = ident.key_hash(inner.store.store_id());
    watched.lock().unwrap().insert(key);
    let cleanup_upload = upload.clone();
    let outcome = match inner.ops.submit(ctx, ident, spec, upload).await {
        Ok(outcome) => outcome,
        Err(error) => {
            release(&cleanup_upload);
            return Err(error);
        }
    };
    Ok((outcome.status, outcome.duplicate))
}

async fn handle_request(
    inner: &Arc<ExecutorInner>,
    session: &Session,
    msg: ExecutorClientMessage,
    out: &Out,
    watched: &Mutex<HashSet<String>>,
) {
    match msg {
        ExecutorClientMessage::Ping(p) => out.send(ExecutorServerMessage::Pong(p)).await,
        ExecutorClientMessage::Describe(req) => {
            let request_id = req.request_id.clone();
            match describe(inner, session, req).await {
                Ok(ok) => out.send(ExecutorServerMessage::DescribeOk(ok)).await,
                Err(err) => out.error(Some(request_id), err).await,
            }
        }
        ExecutorClientMessage::Computer(req) => {
            let required = req.request.required_op();
            let spec = ExecutorOpSpec::Computer { request: req.request };
            let reply = submit(inner, session, required, &req.store_id,
                (&req.run_id, &req.call_id, &req.op_id), spec, None, watched).await;
            send_op_reply(out, req.request_id, reply, true).await;
        }
        ExecutorClientMessage::Exec(req) => {
            let spec = ExecutorOpSpec::from_exec(&req);
            let reply = submit(
                inner,
                session,
                ExecutorOp::Exec,
                &req.store_id,
                (&req.run_id, &req.call_id, &req.op_id),
                spec,
                None,
                watched,
            )
            .await;
            send_op_reply(out, req.request_id, reply, true).await;
        }
        ExecutorClientMessage::Read(req) => {
            let spec = ExecutorOpSpec::from_read(&req);
            let reply = submit(
                inner,
                session,
                ExecutorOp::Read,
                &req.store_id,
                (&req.run_id, &req.call_id, &req.op_id),
                spec,
                None,
                watched,
            )
            .await;
            send_op_reply(out, req.request_id, reply, true).await;
        }
        ExecutorClientMessage::Edit(req) => {
            let spec = ExecutorOpSpec::from_edit(&req);
            let reply = submit(
                inner,
                session,
                ExecutorOp::Edit,
                &req.store_id,
                (&req.run_id, &req.call_id, &req.op_id),
                spec,
                None,
                watched,
            )
            .await;
            send_op_reply(out, req.request_id, reply, true).await;
        }
        ExecutorClientMessage::Cancel(req) => {
            let reply = match authorize(inner, session, ExecutorOp::Cancel, Some(&req.store_id)) {
                Ok((grant, _)) => {
                    let key = lookup_key(inner, &grant, session, &req.op_id);
                    inner
                        .ops
                        .cancel(&key, CancelReason::Client)
                        .await
                        .map(|s| (s, false))
                }
                Err(err) => Err(err),
            };
            send_op_reply(out, req.request_id, reply, false).await;
        }
        ExecutorClientMessage::Query(req) => {
            let reply = match authorize(inner, session, ExecutorOp::Query, Some(&req.store_id)) {
                Ok((grant, _)) => {
                    let key = lookup_key(inner, &grant, session, &req.op_id);
                    inner
                        .ops
                        .query(&key, req.channel, req.offset, req.max_bytes, req.wait_ms)
                        .await
                }
                Err(err) => Err(err),
            };
            match reply {
                Ok((op, chunk)) => {
                    out.send(ExecutorServerMessage::QueryOk(ExecutorQueryReply {
                        request_id: req.request_id,
                        op,
                        chunk,
                    }))
                    .await
                }
                Err(err) => out.error(Some(req.request_id), err).await,
            }
        }
        ExecutorClientMessage::Login(_)
        | ExecutorClientMessage::Write(_)
        | ExecutorClientMessage::UploadEnd(_) => {}
    }
}

async fn describe(
    inner: &Arc<ExecutorInner>,
    session: &Session,
    req: ExecutorDescribeRequest,
) -> Result<ExecutorDescribeOk, ExecutorError> {
    let (grant, _) = authorize(inner, session, ExecutorOp::Describe, None)?;
    let workdir = match req.workdir {
        None => None,
        Some(path) => {
            if !inner.host_caps.contains(&ExecutorOp::Exec) {
                return Err(e(ExecutorErrorCode::Capability, "this host cannot resolve a workdir"));
            }
            if !inner.identity.mode.honors_run_as(&grant.profile) {
                return Err(e(ExecutorErrorCode::Capability, crate::RUN_AS_MISMATCH));
            }
            match run_file_job(
                &inner.launcher,
                &inner.identity,
                grant.profile.run_as.as_ref(),
                &WorkerJob::ResolveDir {
                    path,
                    roots: grant.profile.workdir_roots.clone(),
                },
                &[],
                0,
                None,
            )
            .await
            {
                FileJobOutcome::Reply(reply, _) => match (reply.error, reply.path) {
                    (None, Some(p)) => Some(p),
                    (Some(err), _) => return Err(err),
                    _ => return Err(e(ExecutorErrorCode::Internal, "worker returned no path")),
                },
                FileJobOutcome::SpawnFailed(err) | FileJobOutcome::Unknown(err) => return Err(err),
            }
        }
    };
    Ok(ExecutorDescribeOk {
        request_id: req.request_id,
        target: ExecutorRuntimeTarget {
            node_did: if inner.admission == crate::ExecutorAdmission::Node { inner.node_did.clone() } else { String::new() },
            executor_id: (inner.admission == crate::ExecutorAdmission::Wss).then(|| inner.node_id.clone()),
            profile_id: grant.profile_id.clone(),
            uid: grant.profile.run_as.as_ref().map(|r| r.uid).or_else(|| host_uid(inner)),
            gid: grant.profile.run_as.as_ref().map(|r| r.gid).or_else(|| host_gid(inner)),
            workdir,
            store_id: inner.store.store_id().to_string(),
            protocol_version: NODE_EXECUTOR_PROTOCOL_VERSION,
        },
        info: ExecutorRuntimeInfo {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            hostname: hostname(),
            shell: inner.shell.to_string_lossy().to_string(),
            time_ms: now_ms(),
            path: grant.profile.env.get("PATH").cloned(),
        },
    })
}

fn host_uid(inner: &ExecutorInner) -> Option<u32> {
    #[cfg(unix)]
    { (inner.identity.mode == crate::ExecutorIdentityMode::CurrentUser).then_some(inner.identity.uid) }
    #[cfg(not(unix))]
    { let _ = inner; None }
}

fn host_gid(inner: &ExecutorInner) -> Option<u32> {
    #[cfg(unix)]
    { (inner.identity.mode == crate::ExecutorIdentityMode::CurrentUser).then_some(inner.identity.gid) }
    #[cfg(not(unix))]
    { let _ = inner; None }
}

#[cfg(unix)]
fn hostname() -> String {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return String::new();
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).to_string()
}

#[cfg(not(unix))]
fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

fn begin_upload(
    inner: &Arc<ExecutorInner>,
    session: &Session,
    upload: &mut Option<PendingUpload>,
    req: ExecutorWriteRequest,
) -> Result<(), ExecutorError> {
    if upload.is_some() {
        return Err(e(ExecutorErrorCode::Busy, "another upload is in progress on this stream"));
    }
    let (grant, ctx) = authorize(inner, session, ExecutorOp::Write, Some(&req.store_id))?;
    identity(&grant, session, &req.run_id, &req.call_id, &req.op_id)?;
    if req.content_length > ctx.limits.max_file_bytes {
        return Err(e(ExecutorErrorCode::InvalidArgument, "content_length exceeds max_file_bytes"));
    }
    if !crate::is_host_absolute(&req.path) {
        return Err(e(ExecutorErrorCode::InvalidArgument, "path must be absolute"));
    }
    inner.store.reserve(&ctx.limits, req.content_length)?;
    let path = inner
        .store
        .tmp_dir()
        .join(format!("upload-{}", uuid::Uuid::new_v4().simple()));
    let file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(f) => f,
        Err(err) => {
            inner.store.release(req.content_length);
            return Err(e(ExecutorErrorCode::Internal, format!("cannot stage upload: {err}")));
        }
    };
    *upload = Some(PendingUpload {
        reserved: req.content_length,
        request: req,
        path,
        file,
        received: 0,
        hasher: Sha256::new(),
        deadline: tokio::time::Instant::now() + Duration::from_millis(EXECUTOR_UPLOAD_DEADLINE_MS),
    });
    Ok(())
}

fn on_data(
    _inner: &Arc<ExecutorInner>,
    upload: &mut Option<PendingUpload>,
    data: ExecutorDataFrame,
) -> Result<(), ExecutorError> {
    let Some(pending) = upload.as_mut() else {
        return Ok(());
    };
    if data.op_id != pending.request.op_id {
        return Ok(());
    }
    if data.channel != buckyos_api::ExecutorChannel::Upload || data.offset != pending.received {
        return Err(e(ExecutorErrorCode::ProtocolError, "upload frame out of order"));
    }
    if pending.received + data.data.len() as u64 > pending.request.content_length {
        return Err(e(ExecutorErrorCode::InvalidArgument, "upload exceeds content_length"));
    }
    pending
        .file
        .write_all(&data.data)
        .map_err(|err| e(ExecutorErrorCode::Internal, format!("cannot stage upload: {err}")))?;
    pending.hasher.update(&data.data);
    pending.received += data.data.len() as u64;
    Ok(())
}

fn abort_upload(inner: &ExecutorInner, upload: &mut Option<PendingUpload>) {
    if let Some(pending) = upload.take() {
        drop(pending.file);
        let _ = std::fs::remove_file(&pending.path);
        inner.store.release(pending.reserved);
    }
}

async fn finish_upload(
    inner: &Arc<ExecutorInner>,
    session: &Session,
    pending: PendingUpload,
    watched: &Mutex<HashSet<String>>,
) -> Result<(buckyos_api::ExecutorOpStatus, bool), ExecutorError> {
    let PendingUpload {
        request,
        path,
        file,
        received,
        hasher,
        reserved,
        ..
    } = pending;
    let synced = file.sync_all();
    drop(file);
    let fail = |error: ExecutorError| {
        let _ = std::fs::remove_file(&path);
        inner.store.release(reserved);
        Err(error)
    };
    if let Err(err) = synced {
        return fail(e(ExecutorErrorCode::Internal, format!("cannot stage upload: {err}")));
    }
    if received != request.content_length {
        return fail(e(ExecutorErrorCode::InvalidArgument, "upload is shorter than content_length"));
    }
    let digest = hasher.finalize();
    let hash = format!(
        "sha256:{}",
        digest.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    if hash != request.content_hash {
        return fail(e(ExecutorErrorCode::Conflict, "upload content_hash mismatch"));
    }
    let spec = ExecutorOpSpec::from_write(&request);
    submit(
        inner,
        session,
        ExecutorOp::Write,
        &request.store_id,
        (&request.run_id, &request.call_id, &request.op_id),
        spec,
        Some((path, reserved)),
        watched,
    )
    .await
}

async fn forward_events(
    mut events: tokio::sync::broadcast::Receiver<OpEvent>,
    watched: Arc<Mutex<HashSet<String>>>,
    out: Out,
) {
    loop {
        let event = match events.recv().await {
            Ok(ev) => ev,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        };
        let key = match &event {
            OpEvent::Started { key, .. } | OpEvent::Output { key, .. } | OpEvent::Finished { key, .. } => key,
        };
        if !watched.lock().unwrap().contains(key) {
            continue;
        }
        match event {
            OpEvent::Started { op_id, pid, .. } => {
                out.try_send(ExecutorServerMessage::Started(ExecutorStarted { op_id, pid }))
            }
            OpEvent::Finished { key, op_id, result } => {
                watched.lock().unwrap().remove(&key);
                out.try_send(ExecutorServerMessage::Finished(ExecutorFinished { op_id, result }))
            }
            OpEvent::Output {
                op_id,
                channel,
                offset,
                data,
                ..
            } => {
                for (i, chunk) in data.chunks(EXECUTOR_MAX_DATA_BYTES).enumerate() {
                    let frame = ExecutorDataFrame {
                        op_id: op_id.clone(),
                        channel,
                        offset: offset + (i * EXECUTOR_MAX_DATA_BYTES) as u64,
                        data: chunk.to_vec(),
                    };
                    if let Ok(bytes) = encode_executor_data(&frame) {
                        let _ = out.tx.try_send(bytes);
                    }
                }
            }
        }
    }
}
