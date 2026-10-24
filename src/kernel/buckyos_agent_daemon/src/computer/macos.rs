use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use buckyos_api::*;
use node_executor::{Actor, ComputerContext, ComputerDriver, ComputerOutput};
use serde_json::{json, Value};

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

fn native_capabilities(desktop: &Desktop, paused: bool) -> ComputerCapabilities {
    let state = if paused {
        Some("paused")
    } else {
        desktop_reason(desktop)
    };
    ComputerCapabilities {
        capture: capability(
            true,
            state.or((desktop.capture == 0).then_some("screen_capture_permission_required")),
        ),
        input: capability(
            true,
            state.or((desktop.input == 0).then_some("accessibility_permission_required")),
        ),
        accessibility: capability(false, Some("semantic_observation_unsupported")),
        observation_modes: vec![
            ComputerObservationMode::Visual,
            ComputerObservationMode::Auto,
        ],
        commands: ["click", "type", "keypress", "scroll", "wait_for"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        focus_guard: "focused_element_and_surface".into(),
    }
}

pub fn capabilities() -> Value {
    let desktop = probe();
    let caps = native_capabilities(&desktop, false);
    json!({
        "ui": {"supported": true, "ready": caps.capture.ready && caps.input.ready,
            "state": desktop_reason(&desktop).unwrap_or(if caps.capture.ready && caps.input.ready {"ready"} else {"permission_required"}),
            "backend": "macos_desktop", "minimum_os": "14.0", "display_id": desktop.display,
            "surface_id": format!("display:{}", desktop.display),
            "binding_format": "<authorized_profile_id>:display:<display_id>",
            "interactive_user": desktop.uid, "interactive_session_id": desktop.session,
            "focus_guard": caps.focus_guard, "observation_modes": caps.observation_modes,
            "commands": caps.commands, "text_delivery": ["text"], "scroll_units": ["pixel", "line"]},
        "capture": caps.capture, "input": caps.input, "accessibility": caps.accessibility
    })
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
}

impl ObservationRecord {
    fn expired(&self) -> bool {
        self.created.elapsed() >= OBSERVATION_TTL
            || crate::access::boot_ms().saturating_sub(self.created_boot_ms)
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
        let uid = unsafe { libc::geteuid() };
        let path = std::path::Path::new("/tmp").join(format!("buckyos-computer-{uid}.lock"));
        let mut file = OpenOptions::new()
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
        || !(cleanup < 128 || (256..=258).contains(&cleanup) || cleanup == u16::MAX)
    {
        return Err("desktop_revision_corrupt");
    }
    Ok((
        u64::from_be_bytes(bytes[1..9].try_into().unwrap()),
        bytes[9] == 1,
        cleanup,
    ))
}

impl Drop for DesktopLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

struct MacosComputer {
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

fn new_driver() -> MacosComputer {
    MacosComputer {
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

fn validate_mode(mode: ComputerObservationMode) -> Result<(), &'static str> {
    if matches!(
        mode,
        ComputerObservationMode::Visual | ComputerObservationMode::Auto
    ) {
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
        } => Ok(0),
        ComputerRequest::Type { .. } => Err("text_delivery_unsupported"),
        ComputerRequest::Keypress { keys, .. } => keys
            .last()
            .and_then(|key| resolve_key(key))
            .ok_or("key_unsupported"),
        _ => Ok(u16::MAX),
    }
}

impl MacosComputer {
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

    async fn observe(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        desktop: Desktop,
        epoch: u64,
        revision: u64,
    ) -> Result<(ComputerObservation, Vec<u8>), &'static str> {
        let mode = request.feedback().1;
        validate_mode(mode)?;
        if desktop.capture == 0 {
            return Err("screen_capture_permission_required");
        }
        if let Some(error) = self.stopped(context, epoch).or(desktop_reason(&desktop)) {
            return Err(error);
        }
        if u64::from(desktop.width) * u64::from(desktop.height) > 40_000_000 {
            return Err("capture_pixel_limit");
        }
        let started = node_executor::now_ms();
        let timeout = context.execution_timeout_ms.min(10_000);
        let max_image_bytes = context.max_image_bytes.min(COMPUTER_MAX_IMAGE_BYTES);
        let png = tokio::task::spawn_blocking(move || {
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
                    "screen_capture_permission_required"
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
                    libc::free(bytes.cast());
                }
                return Err("capture_size_limit");
            }
            let image = unsafe { std::slice::from_raw_parts(bytes, length).to_vec() };
            unsafe {
                libc::free(bytes.cast());
            }
            Ok(image)
        })
        .await
        .map_err(|_| "capture_failed")??;
        let after = probe();
        if let Some(error) = self.stopped(context, epoch) {
            return Err(error);
        }
        if !same_surface(&desktop, &after)
            || desktop.events != after.events
            || desktop.focus_pid != after.focus_pid
            || desktop.focus != after.focus
            || after.capture == 0
        {
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
            effective_mode: ComputerObservationMode::Visual,
            selection_reason: if mode == ComputerObservationMode::Auto {
                "semantic_unavailable"
            } else {
                "explicit_request"
            }
            .into(),
            capture_started_at_ms: started,
            capture_completed_at_ms: completed,
            consistency: "best_effort".into(),
            screenshot: ComputerScreenshot {
                artifact_ref: format!("executor-content:{}/{}", context.store_id, context.op_id),
                mime_type: "image/png".into(),
                width: desktop.width,
                height: desktop.height,
                space: ComputerCoordinateSpace::Screenshot,
                interactive: true,
            },
            focus_guard: if desktop.focus != 0 && desktop.focus_on_display != 0 {
                "focused_element_and_surface"
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
            desktop: after,
            epoch,
            revision,
            created: Instant::now(),
            created_boot_ms: crate::access::boot_ms(),
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

    fn target(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        desktop: &Desktop,
        epoch: u64,
        revision: u64,
    ) -> Result<ObservationRecord, &'static str> {
        let target = request.target().ok_or("target_required")?;
        let record = self
            .observations
            .lock()
            .unwrap()
            .get(target.observation_id())
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
        if record.expired()
            || record.epoch != epoch
            || record.revision != revision
            || !same_surface(&record.desktop, desktop)
            || record.desktop.events != desktop.events
        {
            return Err("stale_observation");
        }
        if record.desktop.focus_pid != desktop.focus_pid || record.desktop.focus != desktop.focus {
            return Err("focus_changed");
        }
        if matches!(target, ComputerTarget::Focus { .. })
            && (desktop.focus == 0 || desktop.focus_on_display == 0)
        {
            return Err("focus_unverifiable");
        }
        if let ComputerTarget::Point { x, y, .. } = target {
            point(desktop, *x, *y)?;
        }
        Ok(record)
    }

    async fn act(
        &self,
        context: &ComputerContext,
        request: &ComputerRequest,
        desktop: &Desktop,
        epoch: u64,
        cleanup: u16,
    ) -> Result<(), (&'static str, bool)> {
        let mut dispatched = false;
        let check = |dispatched| -> Result<(), (&'static str, bool)> {
            if let Some(error) = self.stopped(context, epoch) {
                return Err((error, dispatched));
            }
            let current = probe();
            if current.physical_input_down != 0 {
                self.set_paused(true);
                return Err(("human_input_active", dispatched));
            }
            if current.hardware_events != desktop.hardware_events
                || (!dispatched && current.events != desktop.events)
            {
                self.set_paused(true);
                return Err(("human_takeover", dispatched));
            }
            if !same_surface(desktop, &current) || current.input == 0 {
                return Err(("desktop_unavailable", dispatched));
            }
            if (!dispatched || matches!(request.target(), Some(ComputerTarget::Focus { .. })))
                && (desktop.focus_pid != current.focus_pid || desktop.focus != current.focus)
            {
                return Err(("focus_changed", dispatched));
            }
            if matches!(request.target(), Some(ComputerTarget::Focus { .. }))
                && current.focus_on_display == 0
            {
                return Err(("focus_unverifiable", dispatched));
            }
            Ok(())
        };
        match request {
            ComputerRequest::Click {
                target: ComputerTarget::Point { x, y, .. },
                button,
                count,
                modifiers,
                ..
            } => {
                let (x, y) = point(desktop, *x, *y).map_err(|e| (e, false))?;
                for click in 1..=*count {
                    check(dispatched)?;
                    let code = unsafe {
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
                    };
                    if code != 0 {
                        return Err(("input_failed", dispatched));
                    }
                    dispatched = true;
                    if click < *count {
                        tokio::time::sleep(Duration::from_millis(35)).await;
                    }
                }
            }
            ComputerRequest::Type { text, delivery, .. } => {
                if *delivery != ComputerTextDelivery::Text {
                    return Err(("text_delivery_unsupported", false));
                }
                for character in text.chars() {
                    check(dispatched)?;
                    let mut utf16 = [0u16; 2];
                    let character = character.encode_utf16(&mut utf16);
                    if unsafe { bucky_desktop_key(0, 0, character.as_ptr(), character.len()) } != 0
                    {
                        return Err(("input_failed", dispatched));
                    }
                    dispatched = true;
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            }
            ComputerRequest::Keypress { keys, .. } => {
                let code = cleanup;
                check(false)?;
                if unsafe {
                    bucky_desktop_key(code, flags(&keys[..keys.len() - 1]), std::ptr::null(), 0)
                } != 0
                {
                    return Err(("input_failed", false));
                }
            }
            ComputerRequest::Scroll {
                target: ComputerTarget::Point { x, y, .. },
                delta_x,
                delta_y,
                unit,
                ..
            } => {
                let (x, y) = point(desktop, *x, *y).map_err(|e| (e, false))?;
                check(false)?;
                if unsafe {
                    bucky_desktop_scroll(
                        x,
                        y,
                        *delta_x,
                        *delta_y,
                        u32::from(*unit == ComputerScrollUnit::Pixel),
                    )
                } != 0
                {
                    return Err(("input_failed", false));
                }
            }
            ComputerRequest::WaitFor {
                condition: ComputerWaitCondition::Delay { duration_ms },
                ..
            } => {
                let end = Instant::now() + Duration::from_millis(*duration_ms);
                while Instant::now() < end {
                    if let Some(error) = self.stopped(context, epoch) {
                        return Err((error, false));
                    }
                    let current = probe();
                    if current.hardware_events != desktop.hardware_events {
                        self.set_paused(true);
                        return Err(("human_takeover", false));
                    }
                    if !same_surface(desktop, &current) {
                        return Err(("desktop_unavailable", false));
                    }
                    tokio::time::sleep(
                        Duration::from_millis(20)
                            .min(end.saturating_duration_since(Instant::now())),
                    )
                    .await;
                }
            }
            _ => return Err(("command_unsupported", false)),
        }
        Ok(())
    }
}

#[async_trait]
impl ComputerDriver for MacosComputer {
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
            interactive_user: desktop.uid.to_string(),
            interactive_session_id: desktop.session.to_string(),
            desktop_epoch: self.desktop_epoch.clone(),
            computer_session_id: format!("{}:{epoch}", self.desktop_epoch),
            surface_id: format!("display:{}", desktop.display),
            surface_epoch: epoch,
            interaction_resource_id: format!(
                "macos:uid:{}:audit-session:{}",
                desktop.uid, desktop.session
            ),
            expires_at_ms: (node_executor::now_ms() + OBSERVATION_TTL.as_millis() as u64)
                .min(context.auth_valid_until_ms),
        });
        if matches!(request, ComputerRequest::Inspect { .. }) {
            result.capabilities = Some(native_capabilities(
                &desktop,
                self.paused.load(Ordering::Acquire) || lock.dirty,
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
            if let Err(error) = validate_mode(request.feedback().1) {
                return failed(
                    &context,
                    &request,
                    ComputerExecutionState::NotStarted,
                    error,
                );
            }
        }
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
                    "accessibility_permission_required",
                );
            }
            if let Err(error) = self.target(&context, &request, &desktop, epoch, lock.revision) {
                return failed(
                    &context,
                    &request,
                    ComputerExecutionState::NotStarted,
                    error,
                );
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
            let executed = self.act(&context, &request, &desktop, epoch, cleanup).await;
            if let Err(error) = lock.finish() {
                self.set_paused(true);
                return failed(&context, &request, ComputerExecutionState::Unknown, error);
            }
            if let Err((error, dispatched)) = executed {
                return failed(
                    &context,
                    &request,
                    if dispatched {
                        ComputerExecutionState::Partial
                    } else {
                        ComputerExecutionState::NotStarted
                    },
                    error,
                );
            }
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        let mut png = None;
        if request.feedback().0 == ComputerFeedback::After {
            match self
                .observe(&context, &request, probe(), epoch, lock.revision)
                .await
            {
                Ok((observation, image)) => {
                    result.observation = Some(observation);
                    result.feedback.state = ComputerFeedbackState::Included;
                    png = Some(image);
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
        assert_eq!(key_code("Enter"), Some(36));
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
    fn semantic_and_key_text_modes_are_not_advertised() {
        let caps = native_capabilities(&Desktop::default(), false);
        assert!(!caps.accessibility.supported);
        assert!(!caps.capture.ready);
        assert!(!caps.input.ready);
        assert!(validate_mode(ComputerObservationMode::Semantic).is_err());
        assert!(validate_mode(ComputerObservationMode::Hybrid).is_err());
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
            created_boot_ms: crate::access::boot_ms(),
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
        let boot = crate::access::boot_ms();
        if boot > OBSERVATION_TTL.as_millis() as u64 {
            record.created_boot_ms = boot - OBSERVATION_TTL.as_millis() as u64;
            assert!(record.expired());
        }
    }
}
