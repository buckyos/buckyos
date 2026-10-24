use std::ffi::c_void;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf};

type RawHandle = *mut c_void;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: RawHandle,
    inherit: i32,
}

#[repr(C)]
struct TokenUser {
    sid: RawHandle,
    attributes: u32,
}

#[repr(C)]
struct Acl {
    revision: u8,
    reserved: u8,
    size: u16,
    count: u16,
    reserved2: u16,
}

#[repr(C)]
struct Ace {
    kind: u8,
    flags: u8,
    size: u16,
    mask: u32,
    sid: u32,
}

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[repr(C)]
#[derive(Default)]
struct FileInformation {
    attributes: u32,
    created: FileTime,
    accessed: FileTime,
    written: FileTime,
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

#[repr(C)]
#[derive(Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: RawHandle,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> RawHandle;
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
    fn CloseHandle(handle: RawHandle) -> i32;
    fn LocalFree(ptr: RawHandle) -> RawHandle;
    fn GetProcessTimes(
        process: RawHandle,
        created: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn GetFileInformationByHandle(file: RawHandle, info: *mut FileInformation) -> i32;
    fn WaitForSingleObject(handle: RawHandle, timeout: u32) -> u32;
    fn WaitForMultipleObjects(count: u32, handles: *const RawHandle, all: i32, timeout: u32)
        -> u32;
    fn GetTickCount64() -> u64;
    fn CreateDirectoryW(path: *const u16, security: *const SecurityAttributes) -> i32;
    fn LockFileEx(
        file: RawHandle,
        flags: u32,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn CreateEventW(
        security: *const SecurityAttributes,
        manual: i32,
        initial: i32,
        name: *const u16,
    ) -> RawHandle;
    fn OpenEventW(access: u32, inherit: i32, name: *const u16) -> RawHandle;
    fn SetEvent(event: RawHandle) -> i32;
    fn GetLastError() -> u32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: RawHandle, access: u32, token: *mut RawHandle) -> i32;
    fn GetTokenInformation(
        token: RawHandle,
        class: u32,
        info: RawHandle,
        len: u32,
        needed: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: RawHandle, text: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut RawHandle,
        size: *mut u32,
    ) -> i32;
    fn GetSecurityDescriptorDacl(
        descriptor: RawHandle,
        present: *mut i32,
        acl: *mut *mut Acl,
        defaulted: *mut i32,
    ) -> i32;
    fn GetSecurityInfo(
        handle: RawHandle,
        kind: u32,
        info: u32,
        owner: *mut RawHandle,
        group: *mut RawHandle,
        dacl: *mut *mut Acl,
        sacl: *mut RawHandle,
        descriptor: *mut RawHandle,
    ) -> u32;
    fn SetSecurityInfo(
        handle: RawHandle,
        kind: u32,
        info: u32,
        owner: RawHandle,
        group: RawHandle,
        dacl: *mut Acl,
        sacl: RawHandle,
    ) -> u32;
    fn GetAce(acl: *mut Acl, index: u32, ace: *mut RawHandle) -> i32;
}

struct Handle(RawHandle);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Allocation(RawHandle);
impl Drop for Allocation {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn wide(value: &std::ffi::OsStr) -> Result<Vec<u16>, String> {
    let mut text: Vec<_> = value.encode_wide().collect();
    if text.contains(&0) {
        return Err("Windows path or object name contains NUL".into());
    }
    text.push(0);
    Ok(text)
}

fn sid_string(sid: RawHandle) -> Result<String, String> {
    let mut text = std::ptr::null_mut();
    if sid.is_null() || unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err("cannot inspect Windows SID".into());
    }
    let allocation = Allocation(text.cast());
    let mut len = 0;
    unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        let value = String::from_utf16(std::slice::from_raw_parts(text, len))
            .map_err(|_| "invalid Windows SID".to_owned());
        drop(allocation);
        value
    }
}

fn process_sid(process: RawHandle) -> Result<String, String> {
    let mut raw = std::ptr::null_mut();
    if unsafe { OpenProcessToken(process, 0x0008, &mut raw) } == 0 {
        return Err("cannot inspect Windows process identity".into());
    }
    let token = Handle(raw);
    let mut needed = 0;
    unsafe {
        GetTokenInformation(token.0, 1, std::ptr::null_mut(), 0, &mut needed);
    }
    if needed < std::mem::size_of::<TokenUser>() as u32 {
        return Err("cannot inspect Windows token".into());
    }
    let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token.0, 1, buffer.as_mut_ptr().cast(), needed, &mut needed) }
        == 0
    {
        return Err("cannot inspect Windows token".into());
    }
    sid_string(unsafe { (*(buffer.as_ptr().cast::<TokenUser>())).sid })
}

pub fn current_user_sid() -> Result<String, String> {
    process_sid(unsafe { GetCurrentProcess() })
}

pub fn current_session_id() -> Result<u32, String> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(std::process::id(), &mut session) } == 0 {
        return Err("cannot identify current Windows session".into());
    }
    Ok(session)
}

pub fn boot_ms() -> u64 {
    unsafe { GetTickCount64() }
}

pub fn process_start(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let raw = unsafe { OpenProcess(0x0010_1000, 0, pid) };
    if raw.is_null() {
        return None;
    }
    let process = Handle(raw);
    if unsafe { WaitForSingleObject(process.0, 0) } != 258
        || process_sid(process.0).ok()? != current_user_sid().ok()?
    {
        return None;
    }
    let mut created = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    if unsafe { GetProcessTimes(process.0, &mut created, &mut exit, &mut kernel, &mut user) } == 0 {
        return None;
    }
    Some((((created.high as u64) << 32) | created.low as u64).to_string())
}

fn security(inherit: bool) -> Result<Allocation, String> {
    let sid = current_user_sid()?;
    let flags = if inherit { "OICI" } else { "" };
    let sddl = wide(std::ffi::OsStr::new(&format!(
        "O:{sid}D:P(A;{flags};FA;;;{sid})(A;{flags};FA;;;SY)"
    )))?;
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err("cannot construct private Windows ACL".into());
    }
    Ok(Allocation(descriptor))
}

fn attributes(security: &Allocation) -> SecurityAttributes {
    SecurityAttributes {
        length: std::mem::size_of::<SecurityAttributes>() as u32,
        descriptor: security.0,
        inherit: 0,
    }
}

fn open_path(path: &Path, access: u32) -> Result<File, String> {
    let file = OpenOptions::new()
        .access_mode(access)
        .share_mode(3)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "cannot inspect private Windows path")?;
    if file
        .metadata()
        .map_err(|_| "cannot inspect Windows path")?
        .file_attributes()
        & FILE_ATTRIBUTE_REPARSE_POINT
        != 0
    {
        return Err("private Windows paths cannot contain reparse points".into());
    }
    Ok(file)
}

fn path_guards(path: &Path) -> Result<Vec<File>, String> {
    let absolute = std::path::absolute(path).map_err(|_| "invalid Windows path")?;
    let mut current = PathBuf::new();
    let mut guards = Vec::new();
    for component in absolute.components() {
        if component == Component::ParentDir {
            return Err("private Windows paths cannot contain parent traversal".into());
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        guards.push(open_path(&current, FILE_READ_ATTRIBUTES)?);
    }
    Ok(guards)
}

fn inspect_acl(file: &File, require_private: bool) -> Result<(), String> {
    let mut owner = std::ptr::null_mut();
    let mut acl = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            1,
            5,
            &mut owner,
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    } != 0
    {
        return Err("cannot read private Windows ACL".into());
    }
    let _allocation = Allocation(descriptor);
    let sid = current_user_sid()?;
    if sid_string(owner)? != sid {
        return Err("private Windows path belongs to another OS user".into());
    }
    if !require_private {
        return Ok(());
    }
    if acl.is_null() {
        return Err("private Windows path has an unrestricted ACL".into());
    }
    for index in 0..unsafe { (*acl).count } as u32 {
        let mut ace = std::ptr::null_mut();
        if unsafe { GetAce(acl, index, &mut ace) } == 0 {
            return Err("cannot inspect private Windows ACL entry".into());
        }
        let ace = unsafe { &*ace.cast::<Ace>() };
        if ace.kind == 1 {
            continue;
        }
        if ace.kind != 0 || ace.size < 12 {
            return Err("private Windows path has an unsupported ACL entry".into());
        }
        let principal = sid_string((&ace.sid as *const u32).cast_mut().cast())?;
        if principal != sid && principal != "S-1-5-18" {
            return Err("private Windows path grants access to another OS user".into());
        }
    }
    Ok(())
}

pub fn private_dir(path: &Path) -> Result<(), String> {
    let absolute = std::path::absolute(path).map_err(|_| "invalid Windows directory")?;
    let descriptor = security(true)?;
    let security = attributes(&descriptor);
    let mut current = PathBuf::new();
    let mut guards = Vec::new();
    for component in absolute.components() {
        if component == Component::ParentDir {
            return Err("private Windows paths cannot contain parent traversal".into());
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = wide(current.as_os_str())?;
                if unsafe { CreateDirectoryW(name.as_ptr(), &security) } == 0
                    && unsafe { GetLastError() } != 183
                {
                    return Err("cannot create private Windows directory".into());
                }
            }
            Err(_) => return Err("cannot inspect Windows directory".into()),
            Ok(_) => {}
        }
        let guard = open_path(&current, FILE_READ_ATTRIBUTES)?;
        if !guard
            .metadata()
            .map_err(|_| "cannot inspect Windows directory")?
            .is_dir()
        {
            return Err("private Windows directory must be a real directory".into());
        }
        guards.push(guard);
    }
    let file = open_path(&absolute, READ_CONTROL | WRITE_DAC | FILE_READ_ATTRIBUTES)?;
    inspect_acl(&file, false)?;
    let mut acl = std::ptr::null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) }
        == 0
        || present == 0
        || unsafe {
            SetSecurityInfo(
                file.as_raw_handle(),
                1,
                0x8000_0004,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl,
                std::ptr::null_mut(),
            )
        } != 0
    {
        return Err("cannot secure private Windows directory".into());
    }
    inspect_acl(&file, true)
}

pub fn private_file(path: &Path) -> Result<(), String> {
    let _guards = path_guards(path)?;
    let file = open_path(path, READ_CONTROL | FILE_READ_ATTRIBUTES)?;
    private_file_handle(&file)
}

pub fn private_file_handle(file: &File) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|_| "cannot inspect private Windows file")?;
    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("private Windows path must be a regular file".into());
    }
    let mut info = FileInformation::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
        || info.links != 1
    {
        return Err("private Windows file must not have hard links".into());
    }
    inspect_acl(file, true)
}

pub fn read_private_file(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let path = std::path::absolute(path).map_err(|_| "invalid private Windows file path")?;
    let _guards = path_guards(path.parent().ok_or("private Windows file has no parent")?)?;
    let file = OpenOptions::new()
        .access_mode(0x8000_0000 | READ_CONTROL)
        .share_mode(7)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "cannot open private Windows file")?;
    let metadata = file
        .metadata()
        .map_err(|_| "cannot inspect private Windows file")?;
    if !metadata.is_file() || metadata.len() > max {
        return Err("private Windows file is invalid or too large".into());
    }
    private_file_handle(&file)?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read private Windows file")?;
    if bytes.len() as u64 > max {
        return Err("private Windows file is too large".into());
    }
    Ok(bytes)
}

pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    pub fn acquire(path: &Path) -> Result<Self, String> {
        let _guards = path_guards(path.parent().ok_or("lock has no parent")?)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(3)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| "cannot open daemon instance lock")?;
        let metadata = file
            .metadata()
            .map_err(|_| "cannot inspect daemon instance lock")?;
        if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("daemon instance lock must be a regular file".into());
        }
        private_file_handle(&file)?;
        let mut overlapped = Overlapped::default();
        if unsafe { LockFileEx(file.as_raw_handle(), 3, 0, 1, 0, &mut overlapped) } == 0 {
            return Err("another instance is already running".into());
        }
        Ok(Self { _file: file })
    }
}

fn event_name(
    pid: u32,
    start_time: &str,
    user_sid: &str,
    command: &str,
) -> Result<Vec<u16>, String> {
    let sid = current_user_sid()?;
    if user_sid != sid || start_time.parse::<u64>().is_err() {
        return Err("daemon identity does not match current Windows user".into());
    }
    wide(std::ffi::OsStr::new(&format!(
        "Global\\buckyos-agent-daemon-{sid}-{pid}-{start_time}-{command}"
    )))
}

pub struct Controls {
    handles: Vec<Handle>,
}

impl Controls {
    pub fn create(pid: u32, start_time: &str, user_sid: &str) -> Result<Self, String> {
        let descriptor = security(false)?;
        let security = attributes(&descriptor);
        let mut handles = Vec::new();
        for command in ["stop", "pause", "resume"] {
            let name = event_name(pid, start_time, user_sid, command)?;
            let raw = unsafe { CreateEventW(&security, 0, 0, name.as_ptr()) };
            let error = unsafe { GetLastError() };
            if raw.is_null() {
                return Err("cannot create local daemon control event".into());
            }
            let handle = Handle(raw);
            if error == 183 {
                return Err("daemon control event already exists".into());
            }
            handles.push(handle);
        }
        Ok(Self { handles })
    }

    pub fn poll(&self) -> Result<Option<usize>, String> {
        let handles: Vec<_> = self.handles.iter().map(|handle| handle.0).collect();
        match unsafe { WaitForMultipleObjects(handles.len() as u32, handles.as_ptr(), 0, 0) } {
            index @ 0..=2 => Ok(Some(index as usize)),
            258 => Ok(None),
            _ => Err("local daemon control event failed".into()),
        }
    }
}

pub fn signal(pid: u32, start_time: &str, user_sid: &str, command: &str) -> Result<(), String> {
    if process_start(pid).as_deref() != Some(start_time) {
        return Err("daemon exited before the control request".into());
    }
    let name = event_name(pid, start_time, user_sid, command)?;
    let raw = unsafe { OpenEventW(2, 0, name.as_ptr()) };
    if raw.is_null() {
        return Err("cannot open local daemon control event".into());
    }
    let event = Handle(raw);
    if process_start(pid).as_deref() != Some(start_time) || unsafe { SetEvent(event.0) } == 0 {
        return Err("cannot send local daemon control request".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_paths_lock_and_process_identity() {
        let temp = tempfile::tempdir().unwrap();
        private_dir(temp.path()).unwrap();
        let secret = temp.path().join("secret");
        fs::write(&secret, b"secret").unwrap();
        assert_eq!(read_private_file(&secret, 4096).unwrap(), b"secret");
        let lock = temp.path().join("instance.lock");
        let first = InstanceLock::acquire(&lock).unwrap();
        assert!(InstanceLock::acquire(&lock).is_err());
        drop(first);
        assert!(InstanceLock::acquire(&lock).is_ok());
        assert!(current_user_sid().unwrap().starts_with("S-1-"));
        assert!(process_start(std::process::id()).is_some());
        assert!(process_start(0).is_none());
        assert!(process_start(u32::MAX).is_none());
        let alias = temp.path().join("alias");
        fs::hard_link(&secret, &alias).unwrap();
        assert!(read_private_file(&secret, 4096).is_err());
    }

    #[test]
    fn control_events_and_stale_identity() {
        let pid = std::process::id();
        let start_time = process_start(pid).unwrap();
        let sid = current_user_sid().unwrap();
        let controls = Controls::create(pid, &start_time, &sid).unwrap();
        assert!(Controls::create(pid, &start_time, &sid).is_err());
        for (command, index) in [("pause", 1), ("resume", 2), ("stop", 0)] {
            signal(pid, &start_time, &sid, command).unwrap();
            assert_eq!(controls.poll().unwrap(), Some(index));
            assert_eq!(controls.poll().unwrap(), None);
        }
        assert!(signal(pid, "0", &sid, "stop").is_err());
        assert!(signal(pid, &start_time, "S-1-0-0", "stop").is_err());
        assert!(signal(0, &start_time, &sid, "stop").is_err());
        assert_eq!(controls.poll().unwrap(), None);
    }

    #[test]
    fn reparse_paths_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        private_dir(temp.path()).unwrap();
        let real = temp.path().join("real");
        private_dir(&real).unwrap();
        let link = temp.path().join("link");
        match std::os::windows::fs::symlink_dir(&real, &link) {
            Ok(()) => assert!(private_dir(&link).is_err()),
            Err(error) if error.raw_os_error() == Some(1314) => {}
            Err(error) => panic!("cannot create reparse test fixture: {error}"),
        }
    }

    #[test]
    fn public_credentials_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("url");
        fs::write(&path, b"secret").unwrap();
        let sddl = wide(std::ffi::OsStr::new("D:P(A;;FA;;;WD)")).unwrap();
        let mut descriptor = std::ptr::null_mut();
        assert_ne!(
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let descriptor = Allocation(descriptor);
        let mut acl = std::ptr::null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        assert_ne!(
            unsafe {
                GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted)
            },
            0
        );
        let file = open_path(&path, READ_CONTROL | WRITE_DAC).unwrap();
        assert_eq!(
            unsafe {
                SetSecurityInfo(
                    file.as_raw_handle(),
                    1,
                    0x8000_0004,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    acl,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        assert!(private_file(&path).is_err());
    }
}
