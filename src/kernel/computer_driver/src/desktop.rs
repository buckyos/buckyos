use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use buckyos_api::*;
use node_executor::{Actor, ComputerContext, ComputerDriver, ComputerOutput};
use serde_json::{json, Value};

use crate::semantic::{PAT_FOCUSABLE, PAT_INVOKE, PAT_VALUE, ST_DISABLED, ST_READONLY};

const OBSERVATION_TTL: Duration = Duration::from_secs(30);
const MAX_OBSERVATIONS: usize = 64;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Desktop {
    uid: u32,
    session: u32,
    display: u32,
    width: u32,
    height: u32,
    ready: u32,
    capture: u32,
    input: u32,
    locked: u32,
    focus_pid: i32,
    focus_on_display: u32,
    physical_input_down: u32,
    focus: u64,
    events: u64,
    hardware_events: u64,
    layout: u64,
    x: f64,
    y: f64,
    logical_width: f64,
    logical_height: f64,
}

extern "C" {
    fn bucky_desktop_probe(out: *mut Desktop);
    #[cfg(target_os = "macos")]
    fn bucky_desktop_initialize();
    fn bucky_desktop_capture(
        display: u32,
        width: u32,
        height: u32,
        timeout_ms: u64,
        bytes: *mut *mut u8,
        length: *mut usize,
    ) -> i32;
    fn bucky_desktop_click(x: f64, y: f64, button: u32, count: u32, flags: u64) -> i32;
    fn bucky_desktop_key(key: u16, flags: u64, text: *const u16, length: usize) -> i32;
    fn bucky_desktop_keycode(character: u16) -> i32;
    fn bucky_desktop_release(cleanup: u16) -> i32;
    fn bucky_desktop_scroll(x: f64, y: f64, dx: i32, dy: i32, pixel: u32) -> i32;
    #[cfg(target_os = "macos")]
    fn bucky_desktop_request_access(capture: *mut u32, input: *mut u32);
    #[cfg(windows)]
    fn bucky_desktop_free(bytes: *mut std::ffi::c_void);
    #[cfg(windows)]
    fn bucky_desktop_finish() -> i32;
    #[cfg(windows)]
    fn bucky_desktop_set_guard(
        guard: Option<unsafe extern "C" fn(*const std::ffi::c_void, *const Desktop) -> i32>,
        context: *const std::ffi::c_void,
    );
    fn bucky_semantic_status() -> i32;
    fn bucky_semantic_snapshot(
        root: u64,
        root_pid: u32,
        max_nodes: u32,
        budget_ms: u32,
        json: *mut *mut u8,
        length: *mut usize,
    ) -> i32;
    fn bucky_semantic_element(
        snapshot: u64,
        index: u32,
        op: u32,
        text: *const u16,
        text_length: usize,
        timeout_ms: u32,
        json: *mut *mut u8,
        length: *mut usize,
    ) -> i32;
    fn bucky_semantic_retain(snapshot: u64, indices: *const u32, count: usize);
    fn bucky_semantic_release(snapshot: u64);
}

/// Synchronous calls into the native semantic backend (Windows: the UIA
/// worker, macOS: Accessibility); run them in blocking tasks.
mod native_semantic {
    use super::*;

    pub(super) const OP_RESOLVE: u32 = 0;
    pub(super) const OP_FOCUS: u32 = 1;
    pub(super) const OP_INVOKE: u32 = 2;
    pub(super) const OP_SET_VALUE: u32 = 3;

    fn take(code: i32, json: *mut u8, length: usize) -> Result<Vec<u8>, i32> {
        let bytes =
            (!json.is_null()).then(|| unsafe { std::slice::from_raw_parts(json, length).to_vec() });
        unsafe { free_capture(json) };
        if code != 0 {
            return Err(code);
        }
        bytes.ok_or(3)
    }

    pub(super) fn ready() -> bool {
        unsafe { bucky_semantic_status() == 0 }
    }

    pub(super) fn snapshot(
        root: u64,
        root_pid: u32,
        budget_ms: u32,
    ) -> Result<crate::semantic::Snapshot, i32> {
        let (mut json, mut length) = (std::ptr::null_mut(), 0);
        let code = unsafe {
            bucky_semantic_snapshot(
                root,
                root_pid,
                COMPUTER_SEMANTIC_MAX_NODES as u32,
                budget_ms,
                &mut json,
                &mut length,
            )
        };
        serde_json::from_slice(&take(code, json, length)?).map_err(|_| 3)
    }

    pub(super) fn element(
        snapshot: u64,
        index: u32,
        op: u32,
        text: &str,
        timeout_ms: u32,
    ) -> Result<Value, i32> {
        let text: Vec<u16> = text.encode_utf16().collect();
        let (mut json, mut length) = (std::ptr::null_mut(), 0);
        let code = unsafe {
            bucky_semantic_element(
                snapshot,
                index,
                op,
                text.as_ptr(),
                text.len(),
                timeout_ms,
                &mut json,
                &mut length,
            )
        };
        serde_json::from_slice(&take(code, json, length)?).map_err(|_| 3)
    }

    pub(super) fn retain(snapshot: u64, indices: &[u32]) {
        unsafe { bucky_semantic_retain(snapshot, indices.as_ptr(), indices.len()) }
    }

    pub(super) fn release(snapshot: u64) {
        unsafe { bucky_semantic_release(snapshot) }
    }
}

fn semantic_failure(code: i32) -> &'static str {
    match code {
        1 => "stale_element",
        2 | 8 => "backend_timeout",
        4 => "accessibility_unavailable",
        5 => "backend_busy",
        6 => "element_disabled",
        7 => "element_inaccessible",
        _ => "element_action_failed",
    }
}

fn semantic_supported() -> bool {
    native_semantic::ready()
}

const AUTO_MIN_NODES: u32 = 10;
const CHROMIUM_RESAMPLE_MS: u64 = 500;
const AUTO_MIN_ACTIONABLE: usize = 5;

/// Releases the native element handles of one semantic observation.
struct SnapshotHandle(u64);

impl Drop for SnapshotHandle {
    fn drop(&mut self) {
        if self.0 != 0 {
            native_semantic::release(self.0);
        }
    }
}

struct ElementRecord {
    index: u32,
    actions: Vec<ComputerElementAction>,
}

/// ref → backend identity of one semantic observation (Windows: the cached
/// UIA element, RuntimeId, pid and top-level window kept by the worker;
/// macOS: the AXUIElement, its pid, role and window).
struct SemanticRecord {
    handle: SnapshotHandle,
    root: (u64, u32),
    elements: HashMap<String, ElementRecord>,
    windows: HashMap<String, (u64, u32)>,
}

struct SemanticTree {
    payload: ComputerSemantic,
    record: SemanticRecord,
    actionable: usize,
    /// A Chromium/Electron window whose web document is not in the tree yet.
    web_pending: bool,
}

fn sparse_reason(coverage: &ComputerSemanticCoverage, actionable: usize) -> Option<&'static str> {
    if coverage.reason.as_deref() == Some("window_inaccessible") {
        Some("window_inaccessible")
    } else if coverage.returned < AUTO_MIN_NODES || actionable < AUTO_MIN_ACTIONABLE {
        Some("tree_sparse")
    } else {
        None
    }
}

/// `(effective mode, selection_reason)`: `tree` is `Ok` when a rich tree is
/// available, otherwise why not. Auto prefers semantic and falls back to
/// visual; explicit modes keep a sparse tree but fail on a backend error.
fn select_mode(
    mode: ComputerObservationMode,
    tree: Result<(), &'static str>,
) -> Result<(ComputerObservationMode, &'static str), &'static str> {
    use ComputerObservationMode::*;
    match (mode, tree) {
        (Visual, _) => Ok((Visual, "explicit_request")),
        (Auto, Ok(())) => Ok((Semantic, "semantic_available")),
        (Auto, Err(reason)) => Ok((Visual, reason)),
        (_, Ok(()) | Err("tree_sparse" | "window_inaccessible")) => Ok((mode, "explicit_request")),
        (_, Err(error)) => Err(error),
    }
}

/// A native screen rectangle `[left, top, width, height]` in screenshot space.
fn screen_bounds(desktop: &Desktop, rect: [i64; 4]) -> ComputerBounds {
    let scale_x = if desktop.logical_width > 0.0 {
        desktop.width as f64 / desktop.logical_width
    } else {
        1.0
    };
    let scale_y = if desktop.logical_height > 0.0 {
        desktop.height as f64 / desktop.logical_height
    } else {
        1.0
    };
    let clamp_i = |v: f64| v.clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    let clamp_u = |v: f64| v.clamp(0.0, u32::MAX as f64) as u32;
    ComputerBounds {
        x: clamp_i(((rect[0] as f64 - desktop.x) * scale_x).floor()),
        y: clamp_i(((rect[1] as f64 - desktop.y) * scale_y).floor()),
        width: clamp_u((rect[2] as f64 * scale_x).round()),
        height: clamp_u((rect[3] as f64 * scale_y).round()),
    }
}

fn on_display(desktop: &Desktop, x: f64, y: f64) -> bool {
    x.is_finite()
        && y.is_finite()
        && x >= desktop.x
        && y >= desktop.y
        && x < desktop.x + desktop.logical_width
        && y < desktop.y + desktop.logical_height
}

fn build_tree(raw: crate::semantic::Snapshot, desktop: &Desktop, captured_at_ms: u64) -> SemanticTree {
    use crate::semantic::{self, RawNode};
    let nodes: Vec<_> = raw
        .nodes
        .iter()
        .map(|node| node.semantic(screen_bounds(desktop, node.rect())))
        .collect();
    let outline = semantic::outline(&nodes, &semantic::Limits::default());
    let web_pending = raw.chromium()
        && !outline
            .refs
            .iter()
            .any(|(_, i)| nodes[*i].role == ComputerRole::Document);
    let kept: Vec<u32> = outline.refs.iter().map(|(_, i)| *i as u32).collect();
    native_semantic::retain(raw.snapshot, &kept);
    let elements = outline
        .refs
        .iter()
        .map(|(reference, i)| {
            (
                reference.clone(),
                ElementRecord {
                    index: *i as u32,
                    actions: nodes[*i].actions.clone(),
                },
            )
        })
        .collect();
    let mut windows = HashMap::new();
    let list = raw
        .windows
        .iter()
        .take(COMPUTER_MAX_WINDOWS)
        .enumerate()
        .map(|(i, window)| {
            let reference = format!("w{}", i + 1);
            windows.insert(reference.clone(), (window.id, window.pid));
            ComputerWindow {
                window_ref: reference,
                title: semantic::plain(&window.title),
                process: semantic::plain(&window.process),
                pid: window.pid,
                bounds: screen_bounds(desktop, window.bounds),
                foreground: window.foreground,
                accessible: window.accessible,
            }
        })
        .collect();
    let reason = outline
        .truncated
        .map(str::to_owned)
        .or_else(|| (!raw.reason.is_empty()).then(|| raw.reason.clone()));
    SemanticTree {
        payload: ComputerSemantic {
            format: COMPUTER_SEMANTIC_FORMAT.into(),
            text: outline.text,
            windows: list,
            coverage: ComputerSemanticCoverage {
                visited: raw.visited,
                returned: outline.refs.len() as u32,
                truncated: raw.truncated || outline.truncated.is_some(),
                reason,
            },
            captured_at_ms,
        },
        record: SemanticRecord {
            handle: SnapshotHandle(raw.snapshot),
            root: (raw.root, raw.root_pid),
            elements,
            windows,
        },
        actionable: outline.actionable,
        web_pending,
    }
}

/// Where a semantic observation looks: the foreground window, a window the
/// caller chose from a window list, or (action feedback) the acted-on
/// element's window when it still exists.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Scope {
    Foreground,
    Window(u64, u32),
    Prefer(u64, u32),
}

struct Resolved {
    record: Arc<SemanticRecord>,
    index: u32,
    point: (f64, f64),
}

/// Whether the command types into the focused element (focus target, or an
/// element target that is focused first).
fn focus_input(request: &ComputerRequest) -> bool {
    match request.target() {
        Some(ComputerTarget::Focus { .. }) => true,
        Some(ComputerTarget::Element { .. }) => matches!(
            request,
            ComputerRequest::Type { .. } | ComputerRequest::Keypress { .. }
        ),
        _ => false,
    }
}

/// The element target must name an element of that observation and, for
/// semantic commands, one whose observation declared the action.
fn element_target<'a>(
    record: &'a ObservationRecord,
    request: &ComputerRequest,
) -> Result<Option<(&'a Arc<SemanticRecord>, &'a ElementRecord)>, &'static str> {
    let Some(reference) = request.target().and_then(ComputerTarget::element_ref) else {
        return Ok(None);
    };
    let semantic = record.semantic.as_ref().ok_or("stale_element")?;
    let element = semantic.elements.get(reference).ok_or("stale_element")?;
    let required = match request {
        ComputerRequest::Invoke { .. } => Some(ComputerElementAction::Invoke),
        ComputerRequest::SetValue { .. } => Some(ComputerElementAction::SetValue),
        ComputerRequest::Type { .. } | ComputerRequest::Keypress { .. } => {
            Some(ComputerElementAction::Focus)
        }
        _ => None,
    };
    if required.is_some_and(|action| !element.actions.contains(&action)) {
        return Err("element_action_unsupported");
    }
    Ok(Some((semantic, element)))
}

fn element_timeout(context: &ComputerContext) -> u32 {
    context.execution_timeout_ms.clamp(1, 4_000) as u32
}

#[cfg(windows)]
struct NativeInputFence<'a> {
    driver: &'a DesktopComputer,
    context: &'a ComputerContext,
    epoch: u64,
    desktop: &'a Desktop,
    guard_focus: bool,
    require_focus_on_display: bool,
}

#[cfg(windows)]
fn native_focus_matches(expected: &Desktop, current: &Desktop, require_on_display: bool) -> bool {
    expected.focus_pid == current.focus_pid
        && expected.focus == current.focus
        && (!require_on_display || current.focus_on_display != 0)
}

#[cfg(windows)]
unsafe extern "C" fn check_native_fence(
    context: *const std::ffi::c_void,
    current: *const Desktop,
) -> i32 {
    let fence = &*context.cast::<NativeInputFence<'_>>();
    let current = &*current;
    i32::from(
        fence.driver.stopped(fence.context, fence.epoch).is_none()
            && same_surface(fence.desktop, current)
            && (!fence.guard_focus
                || native_focus_matches(fence.desktop, current, fence.require_focus_on_display)),
    )
}

#[cfg(windows)]
struct NativeGuard;

#[cfg(windows)]
impl Drop for NativeGuard {
    fn drop(&mut self) {
        unsafe { bucky_desktop_set_guard(None, std::ptr::null()) };
    }
}

unsafe fn free_capture(bytes: *mut u8) {
    #[cfg(windows)]
    bucky_desktop_free(bytes.cast());
    #[cfg(unix)]
    libc::free(bytes.cast());
}

fn interactive_user(desktop: &Desktop) -> String {
    #[cfg(windows)]
    {
        let _ = desktop;
        crate::windows::current_user_sid().unwrap_or_default()
    }
    #[cfg(unix)]
    desktop.uid.to_string()
}

fn input_reason() -> &'static str {
    if cfg!(windows) {
        "foreground_identity_or_integrity_unavailable"
    } else {
        "accessibility_permission_required"
    }
}

fn accessibility_reason() -> &'static str {
    if cfg!(windows) {
        "ui_automation_unavailable"
    } else {
        "accessibility_permission_required"
    }
}

fn capture_reason() -> &'static str {
    if cfg!(windows) {
        "desktop_capture_unavailable"
    } else {
        "screen_capture_permission_required"
    }
}

fn probe() -> Desktop {
    let mut desktop = Desktop::default();
    unsafe { bucky_desktop_probe(&mut desktop) };
    desktop
}

fn capability(supported: bool, reason: Option<&str>) -> ComputerCapability {
    ComputerCapability {
        supported,
        ready: supported && reason.is_none(),
        reason: reason.map(str::to_owned),
    }
}

fn desktop_reason(desktop: &Desktop) -> Option<&'static str> {
    if desktop.locked != 0 {
        Some("desktop_locked")
    } else if desktop.ready == 0 {
        Some("interactive_session_unavailable")
    } else {
        None
    }
}

fn native_capabilities(desktop: &Desktop, paused: bool, semantic: bool) -> ComputerCapabilities {
    let state = if paused {
        Some("paused")
    } else {
        desktop_reason(desktop)
    };
    let mut observation_modes = vec![
        ComputerObservationMode::Visual,
        ComputerObservationMode::Auto,
    ];
    let mut commands = vec!["click", "type", "keypress", "scroll", "wait_for"];
    if semantic {
        observation_modes.extend([
            ComputerObservationMode::Semantic,
            ComputerObservationMode::Hybrid,
        ]);
        commands.extend(["invoke", "set_value"]);
    }
    ComputerCapabilities {
        capture: capability(
            true,
            state.or((desktop.capture == 0).then_some(capture_reason())),
        ),
        input: capability(
            true,
            state.or((desktop.input == 0).then_some(input_reason())),
        ),
        accessibility: capability(true, state.or((!semantic).then_some(accessibility_reason()))),
        observation_modes,
        commands: commands.into_iter().map(str::to_owned).collect(),
        focus_guard: focus_guard().into(),
    }
}

#[cfg(target_os = "macos")]
pub fn initialize_desktop() {
    unsafe { bucky_desktop_initialize() };
}

/// Ask macOS for Screen Recording and Accessibility access for this process,
/// showing the system prompts. Only for an explicit user request (the
/// Runtime's app entry), never from probes, background tasks or workers.
/// Returns whether (capture, input) are granted now; Screen Recording takes
/// effect after the process restarts.
#[cfg(target_os = "macos")]
pub fn request_access() -> (bool, bool) {
    let (mut capture, mut input) = (0u32, 0u32);
    unsafe { bucky_desktop_request_access(&mut capture, &mut input) };
    (capture != 0, input != 0)
}

pub fn capabilities() -> Value {
    let desktop = probe();
    let caps = native_capabilities(&desktop, false, semantic_supported());
    json!({
        "ui": {"supported": true, "ready": caps.capture.ready && caps.input.ready,
            "state": desktop_reason(&desktop).unwrap_or(if caps.capture.ready && caps.input.ready {"ready"} else {"permission_required"}),
            "backend": if cfg!(windows) {"windows_desktop"} else {"macos_desktop"},
            "minimum_os": if cfg!(windows) {"10.0.15063"} else {"14.0"},
            "display_id": desktop.display,
            "surface_id": format!("display:{}", desktop.display),
            "binding_format": "<authorized_profile_id>:display:<display_id>",
            "interactive_user": if cfg!(windows) {json!(interactive_user(&desktop))} else {json!(desktop.uid)},
            "interactive_session_id": desktop.session,
            "focus_guard": caps.focus_guard, "observation_modes": caps.observation_modes,
            "commands": caps.commands, "text_delivery": ["text"],
            "targets": if caps.accessibility.supported {vec!["point", "focus", "element"]} else {vec!["point", "focus"]},
            "scroll_units": if cfg!(windows) {vec!["line"]} else {vec!["pixel", "line"]}},
        "capture": caps.capture, "input": caps.input, "accessibility": caps.accessibility
    })
}

fn focus_guard() -> &'static str {
    if cfg!(windows) {
        "focused_window_and_surface"
    } else {
        "focused_element_and_surface"
    }
}

#[derive(Clone)]
struct ObservationRecord {
    actor: Actor,
    profile: String,
    store: String,
    server_epoch: String,
    binding: String,
    desktop: Desktop,
    epoch: u64,
    revision: u64,
    created: Instant,
    created_boot_ms: u64,
    semantic: Option<Arc<SemanticRecord>>,
}

impl ObservationRecord {
    fn expired(&self) -> bool {
        self.created.elapsed() >= OBSERVATION_TTL
            || crate::boot_ms().saturating_sub(self.created_boot_ms)
                >= OBSERVATION_TTL.as_millis() as u64
    }

    fn belongs_to(
        &self,
        actor: &Actor,
        profile: &str,
        store: &str,
        server_epoch: &str,
        binding: &str,
    ) -> bool {
        self.actor == *actor
            && self.profile == profile
            && self.store == store
            && self.server_epoch == server_epoch
            && self.binding == binding
    }
}

struct DesktopLock {
    file: File,
    revision: u64,
    dirty: bool,
    cleanup: u16,
}

impl DesktopLock {
    fn try_acquire() -> Result<Option<Self>, &'static str> {
        #[cfg(windows)]
        let mut file = {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            let directory = std::env::var_os("LOCALAPPDATA")
                .map(std::path::PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or("desktop_lock_unavailable")?
                .join("buckyos-agent-daemon")
                .join("computer");
            crate::private_dir(&directory).map_err(|_| "desktop_lock_unavailable")?;
            let session = crate::windows::current_session_id()
                .map_err(|_| "interactive_session_unavailable")?;
            let path = directory.join(format!("session-{session}.lock"));
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                        return Err("desktop_lock_invalid");
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("desktop_lock_unavailable"),
            }
            let file = match OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .custom_flags(0x00200000)
                .open(path)
            {
                Ok(file) => file,
                Err(error) if error.raw_os_error() == Some(32) => return Ok(None),
                Err(_) => return Err("desktop_lock_unavailable"),
            };
            let metadata = file.metadata().map_err(|_| "desktop_lock_unavailable")?;
            if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                return Err("desktop_lock_invalid");
            }
            crate::windows::private_file_handle(&file)
                .map_err(|_| "desktop_lock_invalid")?;
            file
        };
        #[cfg(unix)]
        let mut file = {
            let uid = unsafe { libc::geteuid() };
            let path = std::path::Path::new("/tmp").join(format!("buckyos-computer-{uid}.lock"));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(path)
                .map_err(|_| "desktop_lock_unavailable")?;
            let metadata = file.metadata().map_err(|_| "desktop_lock_unavailable")?;
            if !metadata.is_file()
                || metadata.uid() != uid
                || metadata.mode() & 0o077 != 0
                || metadata.nlink() != 1
            {
                return Err("desktop_lock_invalid");
            }
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                let error = std::io::Error::last_os_error();
                return if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                    Ok(None)
                } else {
                    Err("desktop_lock_unavailable")
                };
            }
            file
        };
        let size = file
            .metadata()
            .map_err(|_| "desktop_lock_unavailable")?
            .len();
        let (revision, dirty, cleanup) = if size == 0 {
            file.write_all(&resource_bytes(0, false, u16::MAX))
                .and_then(|_| file.sync_all())
                .map_err(|_| "desktop_revision_unavailable")?;
            (0, false, u16::MAX)
        } else if size == 12 {
            let mut bytes = [0; 12];
            file.read_exact(&mut bytes)
                .map_err(|_| "desktop_revision_unavailable")?;
            parse_resource(&bytes)?
        } else {
            return Err("desktop_revision_corrupt");
        };
        Ok(Some(Self {
            file,
            revision,
            dirty,
            cleanup,
        }))
    }

    fn begin(&mut self, cleanup: u16) -> Result<(), &'static str> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or("desktop_revision_exhausted")?;
        self.revision = revision;
        self.dirty = true;
        self.cleanup = cleanup;
        self.persist()
    }

    fn persist(&mut self) -> Result<(), &'static str> {
        self.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| {
                self.file
                    .write_all(&resource_bytes(self.revision, self.dirty, self.cleanup))
            })
            .and_then(|_| self.file.sync_all())
            .map_err(|_| "desktop_revision_unavailable")?;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), &'static str> {
        self.dirty = false;
        self.cleanup = u16::MAX;
        self.persist()
    }
}

fn resource_bytes(revision: u64, dirty: bool, cleanup: u16) -> [u8; 12] {
    let mut bytes = [0; 12];
    bytes[0] = 1;
    bytes[1..9].copy_from_slice(&revision.to_be_bytes());
    bytes[9] = u8::from(dirty);
    bytes[10..].copy_from_slice(&cleanup.to_be_bytes());
    bytes
}

fn parse_resource(bytes: &[u8; 12]) -> Result<(u64, bool, u16), &'static str> {
    let cleanup = u16::from_be_bytes(bytes[10..].try_into().unwrap());
    if bytes[0] != 1
        || bytes[9] > 1
        || !(cleanup < if cfg!(windows) { 256 } else { 128 }
            || (256..=if cfg!(windows) { 259 } else { 258 }).contains(&cleanup)
            || cleanup == u16::MAX)
    {
        return Err("desktop_revision_corrupt");
    }
    Ok((
        u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
        bytes[9] == 1,
        cleanup,
    ))
}

#[cfg(unix)]
impl Drop for DesktopLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

struct DesktopComputer {
    serial: tokio::sync::Mutex<()>,
    observations: Mutex<HashMap<String, ObservationRecord>>,
    epoch: AtomicU64,
    paused: AtomicBool,
    generation: AtomicU64,
    desktop_epoch: String,
}

pub fn driver() -> Arc<dyn ComputerDriver> {
    Arc::new(new_driver())
}

fn new_driver() -> DesktopComputer {
    DesktopComputer {
        serial: tokio::sync::Mutex::new(()),
        observations: Mutex::new(HashMap::new()),
        epoch: AtomicU64::new(1),
        paused: AtomicBool::new(false),
        generation: AtomicU64::new(0),
        desktop_epoch: uuid::Uuid::new_v4().to_string(),
    }
}

fn failed(
    context: &ComputerContext,
    request: &ComputerRequest,
    state: ComputerExecutionState,
    code: &str,
) -> ComputerOutput {
    ComputerOutput {
        result: ComputerResult::failed(
            &context.op_id,
            request.binding_id(),
            state,
            code,
            code.replace('_', " "),
        ),
        png: None,
    }
}

fn validate_mode(mode: ComputerObservationMode, semantic: bool) -> Result<(), &'static str> {
    if semantic
        || matches!(
            mode,
            ComputerObservationMode::Visual | ComputerObservationMode::Auto
        )
    {
        Ok(())
    } else {
        Err("observation_mode_unsupported")
    }
}

fn same_surface(a: &Desktop, b: &Desktop) -> bool {
    a.uid == b.uid
        && a.session == b.session
        && a.display == b.display
        && a.layout == b.layout
        && a.width == b.width
        && a.height == b.height
        && a.x == b.x
        && a.y == b.y
        && a.logical_width == b.logical_width
        && a.logical_height == b.logical_height
        && b.ready != 0
}

fn capture_consistent(before: &Desktop, after: &Desktop) -> Result<(), &'static str> {
    if same_surface(before, after) && after.capture != 0 {
        Ok(())
    } else {
        Err("observation_changed_during_capture")
    }
}

/// Whether an action may still use `record` on `desktop`. A person using
/// the mouse or keyboard meanwhile does not invalidate it: only the surface
/// and the focused window the observation saw matter.
fn observation_current(
    record: &ObservationRecord,
    desktop: &Desktop,
    epoch: u64,
    revision: u64,
) -> Result<(), &'static str> {
    if record.expired()
        || record.epoch != epoch
        || record.revision != revision
        || !same_surface(&record.desktop, desktop)
    {
        return Err("stale_observation");
    }
    if record.desktop.focus_pid != desktop.focus_pid || record.desktop.focus != desktop.focus {
        return Err("focus_changed");
    }
    Ok(())
}

fn point(desktop: &Desktop, x: u32, y: u32) -> Result<(f64, f64), &'static str> {
    if x >= desktop.width || y >= desktop.height || desktop.width == 0 || desktop.height == 0 {
        return Err("coordinate_out_of_bounds");
    }
    Ok((
        desktop.x + (x as f64 + 0.5) * desktop.logical_width / desktop.width as f64,
        desktop.y + (y as f64 + 0.5) * desktop.logical_height / desktop.height as f64,
    ))
}

fn flags(modifiers: &[String]) -> u64 {
    modifiers.iter().fold(0, |flags, key| {
        flags
            | match key.as_str() {
                "Shift" => 1 << 17,
                "Control" => 1 << 18,
                "Alt" => 1 << 19,
                "Meta" => 1 << 20,
                _ => 0,
            }
    })
}

#[cfg(unix)]
fn key_code(key: &str) -> Option<u16> {
    Some(match key {
        "Enter" => 36,
        "Tab" => 48,
        "Space" => 49,
        "Backspace" => 51,
        "Escape" => 53,
        "Delete" => 117,
        "Home" => 115,
        "End" => 119,
        "PageUp" => 116,
        "PageDown" => 121,
        "ArrowLeft" => 123,
        "ArrowRight" => 124,
        "ArrowDown" => 125,
        "ArrowUp" => 126,
        "F1" => 122,
        "F2" => 120,
        "F3" => 99,
        "F4" => 118,
        "F5" => 96,
        "F6" => 97,
        "F7" => 98,
        "F8" => 100,
        "F9" => 101,
        "F10" => 109,
        "F11" => 103,
        "F12" => 111,
        _ => return None,
    })
}

#[cfg(windows)]
fn key_code(key: &str) -> Option<u16> {
    Some(match key {
        "Enter" => 0x0d,
        "Tab" => 0x09,
        "Space" => 0x20,
        "Backspace" => 0x08,
        "Escape" => 0x1b,
        "Delete" => 0x2e,
        "Home" => 0x24,
        "End" => 0x23,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "ArrowLeft" => 0x25,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "ArrowUp" => 0x26,
        "F1" => 0x70,
        "F2" => 0x71,
        "F3" => 0x72,
        "F4" => 0x73,
        "F5" => 0x74,
        "F6" => 0x75,
        "F7" => 0x76,
        "F8" => 0x77,
        "F9" => 0x78,
        "F10" => 0x79,
        "F11" => 0x7a,
        "F12" => 0x7b,
        _ => return None,
    })
}

fn resolve_key(key: &str) -> Option<u16> {
    key_code(key).or_else(|| {
        let mut chars = key.chars();
        let character = chars.next()?;
        if chars.next().is_some() || !character.is_ascii_graphic() {
            return None;
        }
        let code = unsafe { bucky_desktop_keycode(character.to_ascii_lowercase() as u16) };
        (code >= 0).then_some(code as u16)
    })
}

fn cleanup_for(request: &ComputerRequest) -> Result<u16, &'static str> {
    match request {
        ComputerRequest::Click { button, .. } => Ok(256
            + match button {
                ComputerButton::Left => 0,
                ComputerButton::Right => 1,
                ComputerButton::Middle => 2,
            }),
        ComputerRequest::Type {
            delivery: ComputerTextDelivery::Text,
            ..
        } => Ok(if cfg!(windows) { 259 } else { 0 }),
        ComputerRequest::Type { .. } => Err("text_delivery_unsupported"),
        ComputerRequest::Keypress { keys, .. } => keys
            .last()
            .and_then(|key| resolve_key(key))
            .ok_or("key_unsupported"),
        ComputerRequest::Scroll {
            unit: ComputerScrollUnit::Pixel,
            ..
        } if cfg!(windows) => Err("scroll_unit_unsupported"),
        _ => Ok(u16::MAX),
    }
}

impl DesktopComputer {
    fn native_input(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        epoch: u64,
        desktop: &Desktop,
        guard_focus: bool,
        call: impl FnOnce() -> i32,
    ) -> i32 {
        #[cfg(windows)]
        let fence = NativeInputFence {
            driver: self,
            context,
            epoch,
            desktop,
            guard_focus,
            require_focus_on_display: focus_input(request),
        };
        #[cfg(windows)]
        unsafe {
            bucky_desktop_set_guard(
                Some(check_native_fence),
                (&fence as *const NativeInputFence<'_>).cast(),
            )
        };
        #[cfg(windows)]
        let _guard = NativeGuard;
        #[cfg(not(windows))]
        let _ = (context, request, epoch, desktop, guard_focus);
        call()
    }

    fn stopped(&self, context: &ComputerContext, epoch: u64) -> Option<&'static str> {
        if context.cancellation.is_timed_out() {
            Some("execution_timeout")
        } else if context.cancellation.is_cancelled() {
            Some("cancelled")
        } else if self.paused.load(Ordering::Acquire) {
            Some("paused")
        } else if self.epoch.load(Ordering::Acquire) != epoch {
            Some("desktop_epoch_changed")
        } else {
            None
        }
    }

    async fn capture(
        &self,
        context: &ComputerContext,
        desktop: &Desktop,
    ) -> Result<Vec<u8>, &'static str> {
        if desktop.capture == 0 {
            return Err(capture_reason());
        }
        if u64::from(desktop.width) * u64::from(desktop.height) > 40_000_000 {
            return Err("capture_pixel_limit");
        }
        let timeout = context.execution_timeout_ms.min(10_000);
        let max_image_bytes = context.max_image_bytes.min(COMPUTER_MAX_IMAGE_BYTES);
        let desktop = *desktop;
        tokio::task::spawn_blocking(move || {
            let mut bytes = std::ptr::null_mut();
            let mut length = 0;
            let code = unsafe {
                bucky_desktop_capture(
                    desktop.display,
                    desktop.width,
                    desktop.height,
                    timeout,
                    &mut bytes,
                    &mut length,
                )
            };
            if code != 0 {
                return Err(if code == 1 {
                    capture_reason()
                } else if code == 2 {
                    "capture_timeout"
                } else if code == 4 {
                    "capture_busy"
                } else {
                    "capture_failed"
                });
            }
            if bytes.is_null() || length == 0 || length > max_image_bytes {
                unsafe {
                    free_capture(bytes);
                }
                return Err("capture_size_limit");
            }
            let image = unsafe { std::slice::from_raw_parts(bytes, length).to_vec() };
            unsafe {
                free_capture(bytes);
            }
            Ok(image)
        })
        .await
        .map_err(|_| "capture_failed")?
    }

    /// The control tree of the scoped window. Chromium enables its renderer
    /// accessibility only after an accessibility client appears (Windows: a
    /// UIA client, macOS: `AXManualAccessibility`), so a first tree of a
    /// Chromium window without its web document is sampled once more.
    async fn semantic_tree(
        &self,
        context: &ComputerContext,
        desktop: &Desktop,
        scope: Scope,
    ) -> Result<SemanticTree, &'static str> {
        let started = Instant::now();
        let budget = COMPUTER_SEMANTIC_SNAPSHOT_MS.min(context.execution_timeout_ms);
        let mut scope = scope;
        for attempt in 0..2 {
            let (root, pid) = match scope {
                Scope::Foreground => (0, 0),
                Scope::Window(root, pid) | Scope::Prefer(root, pid) => (root, pid),
            };
            let raw = tokio::task::spawn_blocking(move || {
                native_semantic::snapshot(root, pid, budget as u32)
            })
            .await
            .map_err(|_| "semantic_failed")?;
            let raw = match raw {
                Err(1) if matches!(scope, Scope::Prefer(..)) => {
                    scope = Scope::Foreground;
                    continue;
                }
                Err(1) => return Err("stale_window"),
                Err(code) => return Err(semantic_failure(code)),
                Ok(raw) => raw,
            };
            let tree = build_tree(raw, desktop, node_executor::now_ms());
            let retry = attempt == 0
                && tree.web_pending
                && started.elapsed() + Duration::from_millis(CHROMIUM_RESAMPLE_MS + budget)
                    < Duration::from_millis(context.execution_timeout_ms);
            if !retry {
                return Ok(tree);
            }
            drop(tree);
            tokio::time::sleep(Duration::from_millis(CHROMIUM_RESAMPLE_MS)).await;
            if let Some(error) = self.stopped(context, self.epoch.load(Ordering::Acquire)) {
                return Err(error);
            }
        }
        Err("semantic_failed")
    }

    async fn observe(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        desktop: Desktop,
        epoch: u64,
        revision: u64,
        scope: Scope,
    ) -> Result<(ComputerObservation, Option<Vec<u8>>), &'static str> {
        let mode = request.feedback().1;
        let semantic = semantic_supported();
        validate_mode(mode, semantic)?;
        if let Some(error) = self.stopped(context, epoch).or(desktop_reason(&desktop)) {
            return Err(error);
        }
        let started = node_executor::now_ms();
        let tree = if mode == ComputerObservationMode::Visual {
            Err("explicit_request")
        } else if !semantic {
            Err("accessibility_unavailable")
        } else {
            self.semantic_tree(context, &desktop, scope).await
        };
        let usable = match &tree {
            Ok(tree) => sparse_reason(&tree.payload.coverage, tree.actionable).map_or(Ok(()), Err),
            Err(error) => Err(*error),
        };
        let (effective, reason) = select_mode(mode, usable)?;
        let tree = if effective == ComputerObservationMode::Visual {
            None
        } else {
            tree.ok()
        };
        let png = if matches!(
            effective,
            ComputerObservationMode::Visual | ComputerObservationMode::Hybrid
        ) {
            Some(self.capture(context, &desktop).await?)
        } else {
            None
        };
        let captured = node_executor::now_ms();
        let after = probe();
        if let Some(error) = self.stopped(context, epoch) {
            return Err(error);
        }
        if png.is_some() {
            capture_consistent(&desktop, &after)?;
        } else if !same_surface(&desktop, &after) {
            return Err("observation_changed_during_capture");
        }
        let observation_id = format!("obs_{}", uuid::Uuid::new_v4().simple());
        let completed = node_executor::now_ms();
        let observation = ComputerObservation {
            observation_id: observation_id.clone(),
            binding_id: request.binding_id().into(),
            surface_id: format!("display:{}", desktop.display),
            surface_epoch: epoch,
            interaction_revision: revision,
            requested_mode: mode,
            effective_mode: effective,
            selection_reason: reason.into(),
            capture_started_at_ms: started,
            capture_completed_at_ms: completed,
            consistency: "best_effort".into(),
            screenshot: png.as_ref().map(|_| ComputerScreenshot {
                artifact_ref: format!("executor-content:{}/{}", context.store_id, context.op_id),
                mime_type: "image/png".into(),
                width: desktop.width,
                height: desktop.height,
                space: ComputerCoordinateSpace::Screenshot,
                interactive: true,
                captured_at_ms: captured,
            }),
            semantic: tree.as_ref().map(|tree| tree.payload.clone()),
            focus_guard: if desktop.focus != 0 && desktop.focus_on_display != 0 {
                focus_guard()
            } else {
                "surface"
            }
            .into(),
            expires_at_ms: (completed + OBSERVATION_TTL.as_millis() as u64)
                .min(context.auth_valid_until_ms),
            warnings: vec![
                "Input acceptance does not verify the application's business outcome.".into(),
            ],
        };
        let record = ObservationRecord {
            actor: context.actor.clone(),
            profile: context.profile_id.clone(),
            store: context.store_id.clone(),
            server_epoch: context.server_epoch.clone(),
            binding: request.binding_id().into(),
            desktop,
            epoch,
            revision,
            created: Instant::now(),
            created_boot_ms: crate::boot_ms(),
            semantic: tree.map(|tree| Arc::new(tree.record)),
        };
        let mut observations = self.observations.lock().unwrap();
        observations.retain(|_, record| !record.expired());
        if observations.len() >= MAX_OBSERVATIONS {
            if let Some(oldest) = observations
                .iter()
                .min_by_key(|(_, record)| record.created)
                .map(|(id, _)| id.clone())
            {
                observations.remove(&oldest);
            }
        }
        observations.insert(observation_id, record);
        Ok((observation, png))
    }

    fn record(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        observation_id: &str,
    ) -> Result<ObservationRecord, &'static str> {
        let record = self
            .observations
            .lock()
            .unwrap()
            .get(observation_id)
            .cloned()
            .ok_or("stale_observation")?;
        if !record.belongs_to(
            &context.actor,
            &context.profile_id,
            &context.store_id,
            &context.server_epoch,
            request.binding_id(),
        ) {
            return Err("observation_forbidden");
        }
        Ok(record)
    }

    /// The window an observe `scope` names in an earlier window list.
    fn scope(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        epoch: u64,
    ) -> Result<Scope, &'static str> {
        let ComputerRequest::Observe {
            scope:
                Some(ComputerObserveScope::Window {
                    observation_id,
                    window_ref,
                }),
            ..
        } = request
        else {
            return Ok(Scope::Foreground);
        };
        let record = self.record(context, request, observation_id)?;
        if record.expired() || record.epoch != epoch {
            return Err("stale_observation");
        }
        record
            .semantic
            .as_ref()
            .and_then(|semantic| semantic.windows.get(window_ref).copied())
            .map(|(hwnd, pid)| Scope::Window(hwnd, pid))
            .ok_or("stale_window")
    }

    fn target(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        desktop: &Desktop,
        epoch: u64,
        revision: u64,
    ) -> Result<ObservationRecord, &'static str> {
        let target = request.target().ok_or("target_required")?;
        let record = self.record(context, request, target.observation_id())?;
        observation_current(&record, desktop, epoch, revision)?;
        if matches!(target, ComputerTarget::Focus { .. })
            && (desktop.focus == 0 || desktop.focus_on_display == 0)
        {
            return Err("focus_unverifiable");
        }
        if let ComputerTarget::Point { x, y, .. } = target {
            point(desktop, *x, *y)?;
        }
        element_target(&record, request)?;
        Ok(record)
    }

    /// Re-resolve an element target through the backend: same native
    /// identity and process, enabled and offering the action, and for
    /// pointer commands on screen, on the display and not covered by
    /// another window. No fallback to a similar node or to coordinates.
    async fn resolve(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        record: &ObservationRecord,
        desktop: &Desktop,
    ) -> Result<Option<Resolved>, &'static str> {
        let Some((semantic, element)) = element_target(record, request)? else {
            return Ok(None);
        };
        let (snapshot, index, timeout) = (semantic.handle.0, element.index, element_timeout(context));
        let state = tokio::task::spawn_blocking(move || {
            native_semantic::element(snapshot, index, native_semantic::OP_RESOLVE, "", timeout)
        })
        .await
        .map_err(|_| "element_action_failed")?
        .map_err(semantic_failure)?;
        let bits = |key: &str| state[key].as_u64().unwrap_or(0) as u32;
        let flag = |key: &str| state[key].as_bool().unwrap_or(true);
        let (states, patterns) = (bits("st"), bits("pat"));
        let rect: Vec<f64> = state["b"]
            .as_array()
            .map(|v| v.iter().filter_map(Value::as_f64).collect())
            .unwrap_or_default();
        let pointer = |x: f64, y: f64, covered: bool| -> Result<(f64, f64), &'static str> {
            if flag("off") {
                Err("element_offscreen")
            } else if !on_display(desktop, x, y) {
                Err("element_out_of_bounds")
            } else if covered {
                Err("element_obscured")
            } else {
                Ok((x, y))
            }
        };
        let required = |pattern: u32| -> Result<(f64, f64), &'static str> {
            if states & ST_DISABLED != 0 {
                Err("element_disabled")
            } else if patterns & pattern == 0 {
                Err("element_action_unsupported")
            } else {
                Ok((0.0, 0.0))
            }
        };
        let point = match request {
            ComputerRequest::Click { .. } => {
                let at: Vec<f64> = state["pt"]
                    .as_array()
                    .map(|v| v.iter().filter_map(Value::as_f64).collect())
                    .unwrap_or_default();
                if at.len() != 2 {
                    return Err("element_out_of_bounds");
                }
                pointer(at[0], at[1], flag("obscured"))?
            }
            ComputerRequest::Scroll { .. } => {
                if rect.len() != 4 {
                    return Err("element_out_of_bounds");
                }
                pointer(
                    (rect[0] + rect[2] / 2.0).floor(),
                    (rect[1] + rect[3] / 2.0).floor(),
                    flag("center_obscured"),
                )?
            }
            ComputerRequest::Type { .. } | ComputerRequest::Keypress { .. } => {
                required(PAT_FOCUSABLE)?
            }
            ComputerRequest::Invoke { .. } => required(PAT_INVOKE)?,
            ComputerRequest::SetValue { .. } => {
                if states & ST_READONLY != 0 {
                    return Err("element_readonly");
                }
                required(PAT_VALUE)?
            }
            _ => return Err("command_unsupported"),
        };
        Ok(Some(Resolved {
            record: semantic.clone(),
            index,
            point,
        }))
    }

    async fn element_op(
        &self,
        context: &ComputerContext,
        resolved: &Resolved,
        op: u32,
        value: &str,
    ) -> Result<Value, (&'static str, ComputerExecutionState)> {
        let (snapshot, index, timeout) =
            (resolved.record.handle.0, resolved.index, element_timeout(context));
        let value = value.to_owned();
        match tokio::task::spawn_blocking(move || {
            native_semantic::element(snapshot, index, op, &value, timeout)
        })
        .await
        {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(2)) => Err(("backend_timeout", ComputerExecutionState::Unknown)),
            Ok(Err(code)) => Err((semantic_failure(code), ComputerExecutionState::NotStarted)),
            Err(_) => Err(("element_action_failed", ComputerExecutionState::Unknown)),
        }
    }

    async fn act(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        desktop: &Desktop,
        epoch: u64,
        cleanup: u16,
        resolved: Option<&Resolved>,
    ) -> Result<(), (&'static str, ComputerExecutionState)> {
        let progress = |dispatched: bool| {
            if dispatched {
                ComputerExecutionState::Partial
            } else {
                ComputerExecutionState::NotStarted
            }
        };
        let focus_input = focus_input(request);
        let check = |reference: &Desktop, dispatched: bool| -> Result<(), (&'static str, ComputerExecutionState)> {
            if let Some(error) = self.stopped(context, epoch) {
                return Err((error, progress(dispatched)));
            }
            let current = probe();
            if !same_surface(reference, &current) || current.input == 0 {
                return Err(("desktop_unavailable", progress(dispatched)));
            }
            if (!dispatched || focus_input)
                && (reference.focus_pid != current.focus_pid || reference.focus != current.focus)
            {
                return Err(("focus_changed", progress(dispatched)));
            }
            if focus_input && current.focus_on_display == 0 {
                return Err(("focus_unverifiable", progress(dispatched)));
            }
            if let Some(error) = self.stopped(context, epoch) {
                return Err((error, progress(dispatched)));
            }
            Ok(())
        };
        let mut dispatched = false;
        let mut focused = None;
        if let (
            Some(resolved),
            ComputerRequest::Type { .. } | ComputerRequest::Keypress { .. },
        ) = (resolved, request)
        {
            if let Some(error) = self.stopped(context, epoch) {
                return Err((error, ComputerExecutionState::NotStarted));
            }
            let current = probe();
            if !same_surface(desktop, &current) || current.input == 0 {
                return Err(("desktop_unavailable", ComputerExecutionState::NotStarted));
            }
            if desktop.focus_pid != current.focus_pid || desktop.focus != current.focus {
                return Err(("focus_changed", ComputerExecutionState::NotStarted));
            }
            let reply = self
                .element_op(context, resolved, native_semantic::OP_FOCUS, "")
                .await?;
            let after = probe();
            if reply["focused"] != Value::Bool(true) || after.focus == 0 || after.focus_on_display == 0
            {
                return Err(("focus_unverifiable", ComputerExecutionState::Partial));
            }
            dispatched = true;
            focused = Some(after);
        }
        let reference = focused.as_ref().unwrap_or(desktop);
        match request {
            ComputerRequest::Click {
                target,
                button,
                count,
                modifiers,
                ..
            } => {
                let (x, y) = match (target, resolved) {
                    (ComputerTarget::Point { x, y, .. }, _) => {
                        point(desktop, *x, *y).map_err(|e| (e, progress(false)))?
                    }
                    (_, Some(resolved)) => resolved.point,
                    _ => return Err(("stale_element", progress(false))),
                };
                for click in 1..=*count {
                    check(desktop, dispatched)?;
                    let code = self.native_input(
                        context,
                        request,
                        epoch,
                        desktop,
                        !dispatched,
                        || unsafe {
                            bucky_desktop_click(
                                x,
                                y,
                                match button {
                                    ComputerButton::Left => 0,
                                    ComputerButton::Right => 1,
                                    ComputerButton::Middle => 2,
                                },
                                u32::from(click),
                                flags(modifiers),
                            )
                        },
                    );
                    if code != 0 {
                        return Err((
                            self.stopped(context, epoch).unwrap_or("input_failed"),
                            progress(dispatched || code == 2),
                        ));
                    }
                    dispatched = true;
                    if click < *count {
                        tokio::time::sleep(Duration::from_millis(35)).await;
                    }
                }
            }
            ComputerRequest::Type { text, delivery, .. } => {
                if *delivery != ComputerTextDelivery::Text {
                    return Err(("text_delivery_unsupported", progress(dispatched)));
                }
                for character in text.chars() {
                    check(reference, dispatched)?;
                    let mut utf16 = [0u16; 2];
                    let character = character.encode_utf16(&mut utf16);
                    let code =
                        self.native_input(context, request, epoch, reference, true, || unsafe {
                            bucky_desktop_key(0, 0, character.as_ptr(), character.len())
                        });
                    if code != 0 {
                        return Err((
                            self.stopped(context, epoch).unwrap_or("input_failed"),
                            progress(dispatched || code == 2),
                        ));
                    }
                    dispatched = true;
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            }
            ComputerRequest::Keypress { keys, .. } => {
                let code = cleanup;
                check(reference, dispatched)?;
                let status = self.native_input(context, request, epoch, reference, true, || unsafe {
                    bucky_desktop_key(code, flags(&keys[..keys.len() - 1]), std::ptr::null(), 0)
                });
                if status != 0 {
                    return Err((
                        self.stopped(context, epoch).unwrap_or("input_failed"),
                        progress(dispatched || status == 2),
                    ));
                }
            }
            ComputerRequest::Scroll {
                target,
                delta_x,
                delta_y,
                unit,
                ..
            } => {
                let (x, y) = match (target, resolved) {
                    (ComputerTarget::Point { x, y, .. }, _) => {
                        point(desktop, *x, *y).map_err(|e| (e, progress(false)))?
                    }
                    (_, Some(resolved)) => resolved.point,
                    _ => return Err(("stale_element", progress(false))),
                };
                check(desktop, false)?;
                let status = self.native_input(context, request, epoch, desktop, true, || unsafe {
                    bucky_desktop_scroll(
                        x,
                        y,
                        *delta_x,
                        *delta_y,
                        u32::from(*unit == ComputerScrollUnit::Pixel),
                    )
                });
                if status != 0 {
                    return Err((
                        self.stopped(context, epoch).unwrap_or("input_failed"),
                        progress(status == 2),
                    ));
                }
            }
            ComputerRequest::Invoke { .. } | ComputerRequest::SetValue { .. } => {
                let resolved = resolved.ok_or(("stale_element", progress(false)))?;
                check(desktop, false)?;
                let (op, value) = match request {
                    ComputerRequest::SetValue { value, .. } => {
                        (native_semantic::OP_SET_VALUE, value.as_str())
                    }
                    _ => (native_semantic::OP_INVOKE, ""),
                };
                let reply = self.element_op(context, resolved, op, value).await?;
                if let ComputerRequest::SetValue { value, .. } = request {
                    if reply
                        .get("v")
                        .and_then(Value::as_str)
                        .is_some_and(|current| current != value)
                    {
                        return Err(("value_mismatch", ComputerExecutionState::Completed));
                    }
                }
            }
            ComputerRequest::WaitFor {
                condition: ComputerWaitCondition::Delay { duration_ms },
                ..
            } => {
                let end = Instant::now() + Duration::from_millis(*duration_ms);
                while Instant::now() < end {
                    if let Some(error) = self.stopped(context, epoch) {
                        return Err((error, progress(false)));
                    }
                    if !same_surface(desktop, &probe()) {
                        return Err(("desktop_unavailable", progress(false)));
                    }
                    tokio::time::sleep(
                        Duration::from_millis(20)
                            .min(end.saturating_duration_since(Instant::now())),
                    )
                    .await;
                }
            }
            _ => return Err(("command_unsupported", progress(false))),
        }
        Ok(())
    }
}

#[async_trait]
impl ComputerDriver for DesktopComputer {
    async fn execute(&self, context: ComputerContext, request: ComputerRequest) -> ComputerOutput {
        if request.validate().is_err() {
            return failed(
                &context,
                &request,
                ComputerExecutionState::NotStarted,
                "invalid_request",
            );
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        let _serial = loop {
            if let Some(error) = self.stopped(&context, epoch) {
                if !matches!(request, ComputerRequest::Inspect { .. }) {
                    return failed(
                        &context,
                        &request,
                        ComputerExecutionState::NotStarted,
                        error,
                    );
                }
            }
            tokio::select! {
                guard = self.serial.lock() => break guard,
                _ = tokio::time::sleep(Duration::from_millis(20)) => {
                    if context.cancellation.is_cancelled() { return failed(&context, &request, ComputerExecutionState::NotStarted, "cancelled"); }
                }
            }
        };
        let mut lock = loop {
            match DesktopLock::try_acquire() {
                Ok(Some(lock)) => break lock,
                Ok(None) => {
                    if context.cancellation.is_cancelled() {
                        return failed(
                            &context,
                            &request,
                            ComputerExecutionState::NotStarted,
                            "desktop_busy_timeout",
                        );
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => {
                    return failed(
                        &context,
                        &request,
                        ComputerExecutionState::NotStarted,
                        error,
                    )
                }
            }
        };
        let desktop = probe();
        let binding = format!("{}:display:{}", context.profile_id, desktop.display);
        if request.binding_id() != binding {
            return failed(
                &context,
                &request,
                ComputerExecutionState::NotStarted,
                "binding_mismatch",
            );
        }
        let mut result =
            ComputerResult::new(&context.op_id, &binding, ComputerExecutionState::Completed);
        result.binding = Some(ComputerBinding {
            binding_id: binding.clone(),
            executor_id: context.executor_id.clone(),
            profile_id: context.profile_id.clone(),
            store_id: context.store_id.clone(),
            origin_generation: self.generation.load(Ordering::Acquire),
            authorized_principal: context.actor.agent_did.clone(),
            interactive_user: interactive_user(&desktop),
            interactive_session_id: desktop.session.to_string(),
            desktop_epoch: self.desktop_epoch.clone(),
            computer_session_id: format!("{}:{epoch}", self.desktop_epoch),
            surface_id: format!("display:{}", desktop.display),
            surface_epoch: epoch,
            interaction_resource_id: if cfg!(windows) {
                format!(
                    "windows:user:{}:session:{}:WinSta0:Default",
                    interactive_user(&desktop),
                    desktop.session
                )
            } else {
                format!(
                    "macos:uid:{}:audit-session:{}",
                    desktop.uid, desktop.session
                )
            },
            expires_at_ms: (node_executor::now_ms() + OBSERVATION_TTL.as_millis() as u64)
                .min(context.auth_valid_until_ms),
        });
        if matches!(request, ComputerRequest::Inspect { .. }) {
            result.capabilities = Some(native_capabilities(
                &desktop,
                self.paused.load(Ordering::Acquire) || lock.dirty,
                semantic_supported(),
            ));
            return ComputerOutput { result, png: None };
        }
        if let Some(error) = self.stopped(&context, epoch).or(desktop_reason(&desktop)) {
            self.invalidate();
            return failed(
                &context,
                &request,
                ComputerExecutionState::NotStarted,
                error,
            );
        }
        if request.feedback().0 == ComputerFeedback::After {
            if let Err(error) = validate_mode(request.feedback().1, semantic_supported()) {
                return failed(
                    &context,
                    &request,
                    ComputerExecutionState::NotStarted,
                    error,
                );
            }
        }
        let mut scope = match self.scope(&context, &request, epoch) {
            Ok(scope) => scope,
            Err(error) => {
                return failed(
                    &context,
                    &request,
                    ComputerExecutionState::NotStarted,
                    error,
                )
            }
        };
        if !matches!(request, ComputerRequest::Observe { .. }) {
            if lock.dirty {
                return failed(
                    &context,
                    &request,
                    ComputerExecutionState::NotStarted,
                    "resource_unknown_requires_local_resume",
                );
            }
            if !matches!(request, ComputerRequest::WaitFor { .. }) && desktop.input == 0 {
                return failed(
                    &context,
                    &request,
                    ComputerExecutionState::NotStarted,
                    input_reason(),
                );
            }
            let record = match self.target(&context, &request, &desktop, epoch, lock.revision) {
                Ok(record) => record,
                Err(error) => {
                    return failed(
                        &context,
                        &request,
                        ComputerExecutionState::NotStarted,
                        error,
                    )
                }
            };
            let resolved = match self.resolve(&context, &request, &record, &desktop).await {
                Ok(resolved) => resolved,
                Err(error) => {
                    return failed(
                        &context,
                        &request,
                        ComputerExecutionState::NotStarted,
                        error,
                    )
                }
            };
            if let Some(resolved) = &resolved {
                let (root, pid) = resolved.record.root;
                scope = Scope::Prefer(root, pid);
            }
            let cleanup = match cleanup_for(&request) {
                Ok(cleanup) => cleanup,
                Err(error) => {
                    return failed(
                        &context,
                        &request,
                        ComputerExecutionState::NotStarted,
                        error,
                    )
                }
            };
            if let Err(error) = lock.begin(cleanup) {
                return failed(
                    &context,
                    &request,
                    ComputerExecutionState::NotStarted,
                    error,
                );
            }
            self.observations.lock().unwrap().clear();
            let executed = self
                .act(&context, &request, &desktop, epoch, cleanup, resolved.as_ref())
                .await;
            #[cfg(windows)]
            if unsafe { bucky_desktop_finish() } != 0 {
                self.set_paused(true);
                return failed(
                    &context,
                    &request,
                    ComputerExecutionState::Unknown,
                    "input_release_failed",
                );
            }
            if let Err(error) = lock.finish() {
                self.set_paused(true);
                return failed(&context, &request, ComputerExecutionState::Unknown, error);
            }
            if let Err((error, state)) = executed {
                return failed(&context, &request, state, error);
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        let mut png = None;
        if request.feedback().0 == ComputerFeedback::After {
            match self
                .observe(&context, &request, probe(), epoch, lock.revision, scope)
                .await
            {
                Ok((observation, image)) => {
                    result.observation = Some(observation);
                    result.feedback.state = ComputerFeedbackState::Included;
                    png = image;
                }
                Err(error) => {
                    if matches!(request, ComputerRequest::Observe { .. }) {
                        result.execution.state = ComputerExecutionState::NotStarted;
                    }
                    result.feedback.state = ComputerFeedbackState::Failed;
                    result.error = Some(ComputerError {
                        code: error.into(),
                        message: error.replace('_', " "),
                    });
                }
            }
        }
        ComputerOutput { result, png }
    }

    fn invalidate(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.observations.lock().unwrap().clear();
    }

    fn set_paused(&self, paused: bool) {
        self.paused.store(true, Ordering::Release);
        self.invalidate();
        if paused {
            return;
        }
        let Ok(_serial) = self.serial.try_lock() else {
            return;
        };
        let Ok(Some(mut lock)) = DesktopLock::try_acquire() else {
            return;
        };
        if lock.dirty {
            let desktop = probe();
            if desktop.ready == 0
                || desktop.input == 0
                || desktop.physical_input_down != 0
                || unsafe { bucky_desktop_release(lock.cleanup) } != 0
                || lock.finish().is_err()
            {
                return;
            }
        }
        self.paused.store(false, Ordering::Release);
    }

    fn set_generation(&self, generation: u64) {
        self.generation.store(generation, Ordering::Release);
        self.invalidate();
    }

    fn is_paused(&self) -> bool {
        if self.paused.load(Ordering::Acquire) {
            return true;
        }
        let Ok(_serial) = self.serial.try_lock() else {
            return false;
        };
        match DesktopLock::try_acquire() {
            Ok(Some(lock)) => lock.dirty,
            Ok(None) => false,
            Err(_) => true,
        }
    }

    fn capabilities(&self) -> Value {
        let mut value = capabilities();
        if self.is_paused() {
            value["ui"]["ready"] = false.into();
            value["ui"]["state"] = "paused".into();
            value["input"]["ready"] = false.into();
            value["input"]["reason"] = "paused".into();
            value["capture"]["ready"] = false.into();
            value["capture"]["reason"] = "paused".into();
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retina_coordinates_and_negative_display_origin() {
        let desktop = Desktop {
            x: -1440.0,
            y: 120.0,
            width: 2880,
            height: 1800,
            logical_width: 1440.0,
            logical_height: 900.0,
            ..Desktop::default()
        };
        assert_eq!(point(&desktop, 0, 0).unwrap(), (-1439.75, 120.25));
        assert_eq!(point(&desktop, 2879, 1799).unwrap(), (-0.25, 1019.75));
        assert!(point(&desktop, 2880, 0).is_err());
        assert!(point(&desktop, 0, 1800).is_err());
    }

    #[test]
    fn modifiers_preserve_control_and_command() {
        assert_ne!(flags(&["Control".into()]), flags(&["Meta".into()]));
        assert_eq!(
            flags(&["Control".into(), "Meta".into()]),
            (1 << 18) | (1 << 20)
        );
        assert_eq!(
            key_code("Enter"),
            Some(if cfg!(windows) { 0x0d } else { 36 })
        );
        assert_eq!(key_code("Meta"), None);
        assert_eq!(key_code("a"), None);
    }

    #[test]
    fn desktop_switch_layout_and_lock_invalidate_surface() {
        let desktop = Desktop {
            ready: 1,
            display: 1,
            uid: 501,
            session: 2,
            layout: 7,
            ..Desktop::default()
        };
        assert!(same_surface(&desktop, &desktop));
        assert!(!same_surface(
            &desktop,
            &Desktop {
                session: 3,
                ..desktop
            }
        ));
        assert!(!same_surface(
            &desktop,
            &Desktop {
                layout: 8,
                ..desktop
            }
        ));
        assert!(!same_surface(
            &desktop,
            &Desktop {
                ready: 0,
                ..desktop
            }
        ));
    }

    #[test]
    fn capture_survives_input_and_focus_but_not_surface_change() {
        let before = Desktop {
            ready: 1,
            capture: 1,
            display: 1,
            session: 2,
            layout: 7,
            width: 1920,
            height: 1080,
            focus_pid: 100,
            focus: 200,
            events: 10,
            hardware_events: 10,
            ..Desktop::default()
        };
        let busy = Desktop {
            focus_pid: 101,
            focus: 201,
            events: 15,
            hardware_events: 15,
            ..before
        };
        assert!(capture_consistent(&before, &busy).is_ok());
        assert!(capture_consistent(&before, &Desktop { layout: 8, ..busy }).is_err());
        assert!(capture_consistent(&before, &Desktop { capture: 0, ..busy }).is_err());
    }

    #[test]
    fn human_input_does_not_invalidate_an_observation() {
        let seen = Desktop {
            ready: 1,
            display: 1,
            session: 2,
            layout: 7,
            focus_pid: 100,
            focus: 200,
            events: 10,
            hardware_events: 10,
            ..Desktop::default()
        };
        let record = ObservationRecord {
            actor: Actor {
                agent_did: "did:example:alice".into(),
                app_instance_id: "app@alice".into(),
            },
            profile: "desktop".into(),
            store: "store-1".into(),
            server_epoch: "epoch-1".into(),
            binding: "desktop:display:1".into(),
            desktop: seen,
            epoch: 1,
            revision: 1,
            created: Instant::now(),
            created_boot_ms: crate::boot_ms(),
            semantic: None,
        };
        let used = Desktop {
            events: 90,
            hardware_events: 80,
            physical_input_down: 1,
            ..seen
        };
        assert_eq!(observation_current(&record, &used, 1, 1), Ok(()));
        assert_eq!(
            observation_current(&record, &Desktop { focus: 201, ..used }, 1, 1),
            Err("focus_changed")
        );
        assert_eq!(
            observation_current(&record, &Desktop { layout: 8, ..used }, 1, 1),
            Err("stale_observation")
        );
        assert_eq!(observation_current(&record, &used, 2, 1), Err("stale_observation"));
        assert_eq!(observation_current(&record, &used, 1, 2), Err("stale_observation"));
    }

    #[test]
    fn semantic_modes_and_commands_follow_the_backend() {
        let caps = native_capabilities(&Desktop::default(), false, false);
        assert!(caps.accessibility.supported);
        assert!(!caps.accessibility.ready);
        assert!(!caps.capture.ready);
        assert!(!caps.input.ready);
        assert!(!caps.commands.contains(&"invoke".to_string()));
        assert!(validate_mode(ComputerObservationMode::Semantic, false).is_err());
        assert!(validate_mode(ComputerObservationMode::Hybrid, false).is_err());
        assert!(validate_mode(ComputerObservationMode::Auto, false).is_ok());
        assert!(validate_mode(ComputerObservationMode::Hybrid, true).is_ok());
        let ready = Desktop {
            ready: 1,
            capture: 1,
            input: 1,
            ..Desktop::default()
        };
        let caps = native_capabilities(&ready, false, true);
        assert!(caps.accessibility.ready);
        assert!(caps
            .observation_modes
            .contains(&ComputerObservationMode::Semantic));
        assert!(caps.commands.contains(&"set_value".to_string()));
        assert_eq!(
            native_capabilities(&ready, true, true)
                .accessibility
                .reason
                .as_deref(),
            Some("paused")
        );
        assert_eq!(
            native_capabilities(&ready, false, false)
                .accessibility
                .reason
                .as_deref(),
            Some(accessibility_reason())
        );
    }

    fn coverage(returned: u32, reason: Option<&str>) -> ComputerSemanticCoverage {
        ComputerSemanticCoverage {
            visited: returned,
            returned,
            truncated: false,
            reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn auto_prefers_a_rich_tree_and_explains_fallbacks() {
        use ComputerObservationMode::*;
        assert_eq!(sparse_reason(&coverage(40, None), 12), None);
        assert_eq!(sparse_reason(&coverage(40, Some("timeout")), 12), None);
        assert_eq!(sparse_reason(&coverage(6, None), 12), Some("tree_sparse"));
        assert_eq!(sparse_reason(&coverage(40, None), 2), Some("tree_sparse"));
        assert_eq!(
            sparse_reason(&coverage(0, Some("window_inaccessible")), 0),
            Some("window_inaccessible")
        );
        assert_eq!(
            select_mode(Auto, Ok(())),
            Ok((Semantic, "semantic_available"))
        );
        for reason in [
            "tree_sparse",
            "window_inaccessible",
            "accessibility_unavailable",
            "backend_timeout",
        ] {
            assert_eq!(select_mode(Auto, Err(reason)), Ok((Visual, reason)));
        }
        assert_eq!(select_mode(Visual, Ok(())), Ok((Visual, "explicit_request")));
        assert_eq!(
            select_mode(Semantic, Err("tree_sparse")),
            Ok((Semantic, "explicit_request"))
        );
        assert_eq!(select_mode(Hybrid, Ok(())), Ok((Hybrid, "explicit_request")));
        assert_eq!(
            select_mode(Semantic, Err("backend_timeout")),
            Err("backend_timeout")
        );
        assert_eq!(select_mode(Hybrid, Err("stale_window")), Err("stale_window"));
    }

    fn element_record(semantic: Option<Arc<SemanticRecord>>) -> ObservationRecord {
        ObservationRecord {
            actor: Actor {
                agent_did: "did:example:alice".into(),
                app_instance_id: "app@alice".into(),
            },
            profile: "desktop".into(),
            store: "store-1".into(),
            server_epoch: "epoch-1".into(),
            binding: "desktop:display:1".into(),
            desktop: Desktop::default(),
            epoch: 1,
            revision: 1,
            created: Instant::now(),
            created_boot_ms: crate::boot_ms(),
            semantic,
        }
    }

    fn element_request(command: &str, reference: &str) -> ComputerRequest {
        let mut value = json!({"command": command, "binding_id": "desktop:display:1",
            "target": {"kind": "element", "observation_id": "obs", "ref": reference}});
        match command {
            "set_value" => value["value"] = json!("x"),
            "type" => value["text"] = json!("x"),
            _ => {}
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn element_refs_resolve_only_within_their_observation_and_declared_actions() {
        let semantic = Arc::new(SemanticRecord {
            handle: SnapshotHandle(0),
            root: (10, 20),
            elements: HashMap::from([(
                "e2".to_string(),
                ElementRecord {
                    index: 7,
                    actions: vec![ComputerElementAction::Focus, ComputerElementAction::Invoke],
                },
            )]),
            windows: HashMap::from([("w1".to_string(), (10, 20))]),
        });
        let record = element_record(Some(semantic));
        let (found, element) = element_target(&record, &element_request("invoke", "e2"))
            .unwrap()
            .unwrap();
        assert_eq!((element.index, found.root), (7, (10, 20)));
        assert!(element_target(&record, &element_request("click", "e2"))
            .unwrap()
            .is_some());
        assert!(element_target(&record, &element_request("type", "e2"))
            .unwrap()
            .is_some());
        assert_eq!(
            element_target(&record, &element_request("set_value", "e2")).err(),
            Some("element_action_unsupported")
        );
        assert_eq!(
            element_target(&record, &element_request("invoke", "e3")).err(),
            Some("stale_element")
        );
        assert_eq!(
            element_target(&element_record(None), &element_request("invoke", "e2")).err(),
            Some("stale_element")
        );
        assert!(focus_input(&element_request("type", "e2")));
        assert!(!focus_input(&element_request("click", "e2")));
        assert!(!focus_input(&element_request("invoke", "e2")));
    }

    #[test]
    fn native_bounds_map_into_screenshot_space() {
        let desktop = Desktop {
            x: -1920.0,
            y: 0.0,
            width: 2880,
            height: 1800,
            logical_width: 1440.0,
            logical_height: 900.0,
            ..Desktop::default()
        };
        assert_eq!(
            screen_bounds(&desktop, [-1920, 0, 100, 50]),
            ComputerBounds {
                x: 0,
                y: 0,
                width: 200,
                height: 100
            }
        );
        assert_eq!(screen_bounds(&desktop, [-1930, -5, 10, 10]).x, -20);
        assert!(on_display(&desktop, -1920.0, 0.0));
        assert!(!on_display(&desktop, -480.0, 10.0));
        assert!(!on_display(&desktop, f64::NAN, 10.0));
    }

    #[test]
    fn unfinished_input_survives_lock_owner_exit() {
        let unfinished = resource_bytes(4, true, 36);
        assert_eq!(parse_resource(&unfinished).unwrap(), (4, true, 36));
        let released = resource_bytes(4, false, u16::MAX);
        assert_eq!(parse_resource(&released).unwrap(), (4, false, u16::MAX));
        let mut corrupt = unfinished;
        corrupt[0] = 2;
        assert!(parse_resource(&corrupt).is_err());
        corrupt = unfinished;
        corrupt[9] = 2;
        assert!(parse_resource(&corrupt).is_err());
        assert!(parse_resource(&resource_bytes(4, true, 500)).is_err());
    }

    #[test]
    fn pause_resume_and_reconnect_fence_existing_execution() {
        let driver = new_driver();
        let first = driver.epoch.load(Ordering::Acquire);
        driver.set_generation(12);
        assert!(driver.epoch.load(Ordering::Acquire) > first);
        assert_eq!(driver.generation.load(Ordering::Acquire), 12);
        let _busy = driver.serial.try_lock().unwrap();
        driver.set_paused(true);
        let paused_epoch = driver.epoch.load(Ordering::Acquire);
        driver.set_paused(false);
        assert!(driver.is_paused());
        assert!(driver.epoch.load(Ordering::Acquire) > paused_epoch);
    }

    #[test]
    fn observations_are_bound_to_actor_store_and_connection_and_expire_after_sleep() {
        let actor = Actor {
            agent_did: "did:example:alice".into(),
            app_instance_id: "app@alice".into(),
        };
        let mut record = ObservationRecord {
            actor: actor.clone(),
            profile: "desktop".into(),
            store: "store-1".into(),
            server_epoch: "epoch-1".into(),
            binding: "desktop:display:1".into(),
            desktop: Desktop::default(),
            epoch: 1,
            revision: 1,
            created: Instant::now(),
            created_boot_ms: crate::boot_ms(),
            semantic: None,
        };
        assert!(record.belongs_to(&actor, "desktop", "store-1", "epoch-1", "desktop:display:1"));
        let other = Actor {
            agent_did: "did:example:bob".into(),
            ..actor.clone()
        };
        assert!(!record.belongs_to(&other, "desktop", "store-1", "epoch-1", "desktop:display:1"));
        assert!(!record.belongs_to(&actor, "desktop", "store-2", "epoch-1", "desktop:display:1"));
        assert!(!record.belongs_to(&actor, "desktop", "store-1", "epoch-2", "desktop:display:1"));
        assert!(!record.expired());
        record.created = Instant::now() - OBSERVATION_TTL;
        assert!(record.expired());
        record.created = Instant::now();
        let boot = crate::boot_ms();
        if boot > OBSERVATION_TTL.as_millis() as u64 {
            record.created_boot_ms = boot - OBSERVATION_TTL.as_millis() as u64;
            assert!(record.expired());
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_final_focus_guard_rejects_window_moved_to_another_display() {
        let observed = Desktop {
            ready: 1,
            display: 1,
            session: 1,
            layout: 7,
            focus_pid: 100,
            focus: 200,
            focus_on_display: 1,
            ..Desktop::default()
        };
        let moved = Desktop {
            focus_on_display: 0,
            ..observed
        };
        assert!(same_surface(&observed, &moved));
        assert!(native_focus_matches(&observed, &observed, true));
        assert!(!native_focus_matches(&observed, &moved, true));
        assert!(native_focus_matches(&observed, &moved, false));
        assert!(!native_focus_matches(
            &observed,
            &Desktop {
                focus: 201,
                ..observed
            },
            false
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_physical_coordinates_preserve_dpi_and_display_origin() {
        let desktop = Desktop {
            x: -1920.0,
            y: -1080.0,
            width: 1920,
            height: 1080,
            logical_width: 1920.0,
            logical_height: 1080.0,
            ..Desktop::default()
        };
        assert_eq!(point(&desktop, 0, 0).unwrap(), (-1919.5, -1079.5));
        assert_eq!(point(&desktop, 1919, 1079).unwrap(), (-0.5, -0.5));
        assert!(point(&desktop, 1920, 100).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_cleanup_and_capabilities_match_native_input() {
        assert_eq!(
            parse_resource(&resource_bytes(3, true, 259)).unwrap(),
            (3, true, 259)
        );
        assert!(parse_resource(&resource_bytes(3, true, 260)).is_err());
        assert_eq!(key_code("Delete"), Some(0x2e));
        assert_eq!(key_code("F12"), Some(0x7b));
        let request: ComputerRequest = serde_json::from_value(json!({
            "command": "scroll", "binding_id": "profile:display:1",
            "target": {"kind":"point", "observation_id":"obs", "x":0, "y":0, "space":"screenshot"},
            "delta_x":0, "delta_y":1, "unit":"pixel"
        }))
        .unwrap();
        assert_eq!(cleanup_for(&request), Err("scroll_unit_unsupported"));
        let desktop = Desktop {
            ready: 1,
            capture: 1,
            input: 0,
            ..Desktop::default()
        };
        let caps = native_capabilities(&desktop, false, false);
        assert!(caps.capture.ready);
        assert_eq!(
            caps.input.reason.as_deref(),
            Some("foreground_identity_or_integrity_unavailable")
        );
        assert!(caps.accessibility.supported);
        assert_eq!(
            caps.accessibility.reason.as_deref(),
            Some("ui_automation_unavailable")
        );
        assert!(native_capabilities(&desktop, false, true).accessibility.ready);
        assert!(!native_capabilities(&desktop, true, true).capture.ready);
    }

    /// `BUCKY_SEMANTIC_TITLE=<title substring>` picks a window from the
    /// window list instead of the foreground window.
    #[test]
    #[ignore = "requires an unlocked interactive desktop with UI Automation / Accessibility; reads the control tree without input"]
    fn semantic_smoke() {
        let desktop = probe();
        assert_eq!(desktop.ready, 1, "desktop is unavailable: {desktop:?}");
        assert!(native_semantic::ready(), "the accessibility backend is unavailable");
        let first = native_semantic::snapshot(0, 0, 3000).expect("foreground snapshot");
        let scope = std::env::var("BUCKY_SEMANTIC_TITLE").ok().map(|title| {
            first
                .windows
                .iter()
                .find(|w| w.title.contains(&title))
                .map(|w| (w.id, w.pid))
                .expect("no listed window has that title")
        });
        for attempt in 0..2 {
            let started = Instant::now();
            let raw = match scope {
                Some((root, pid)) => native_semantic::snapshot(root, pid, 3000).expect("window snapshot"),
                None => native_semantic::snapshot(0, 0, 3000).expect("foreground snapshot"),
            };
            let elapsed = started.elapsed();
            let tree = build_tree(raw, &desktop, 0);
            let payload = serde_json::to_vec(&tree.payload).unwrap().len();
            if attempt == 1 {
                for window in &tree.payload.windows {
                    println!("{} {:?} {} foreground={} accessible={}", window.window_ref, window.title, window.process, window.foreground, window.accessible);
                }
                println!("{}", tree.payload.text);
            }
            println!(
                "attempt {attempt}: web_pending={} coverage={:?} actionable={} sparse={:?} outline_bytes={} payload_bytes={payload} elapsed={elapsed:?}",
                tree.web_pending,
                tree.payload.coverage,
                tree.actionable,
                sparse_reason(&tree.payload.coverage, tree.actionable),
                tree.payload.text.len(),
            );
            assert!(tree.payload.within_limit());
            assert!(elapsed < Duration::from_millis(5_600));
            if tree.web_pending {
                std::thread::sleep(Duration::from_millis(CHROMIUM_RESAMPLE_MS));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an unlocked Windows interactive desktop; captures without input"]
    fn windows_desktop_capture_smoke() {
        let desktop = probe();
        assert_eq!(desktop.ready, 1, "desktop is unavailable: {desktop:?}");
        assert_eq!(desktop.capture, 1, "capture is unavailable: {desktop:?}");
        assert_eq!(
            desktop.session,
            crate::windows::current_session_id().unwrap()
        );
        assert_eq!(desktop.width as f64, desktop.logical_width);
        assert_eq!(desktop.height as f64, desktop.logical_height);
        let mut bytes = std::ptr::null_mut();
        let mut length = 0;
        let status = unsafe {
            bucky_desktop_capture(
                desktop.display,
                desktop.width,
                desktop.height,
                10000,
                &mut bytes,
                &mut length,
            )
        };
        assert_eq!(status, 0, "native screenshot failed");
        assert!(!bytes.is_null() && length >= 24 && length <= COMPUTER_MAX_IMAGE_BYTES);
        let png = unsafe { std::slice::from_raw_parts(bytes, length).to_vec() };
        unsafe { free_capture(bytes) };
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            desktop.width
        );
        assert_eq!(
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
            desktop.height
        );
        let resource = DesktopLock::try_acquire().unwrap().expect("desktop busy");
        assert!(DesktopLock::try_acquire().unwrap().is_none());
        assert!(
            !resource.dirty,
            "desktop has unfinished input requiring local resume"
        );
        drop(resource);
        assert!(DesktopLock::try_acquire().unwrap().is_some());
    }
}
