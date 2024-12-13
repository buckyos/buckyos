use futures::FutureExt;
use std::collections::{HashMap, HashSet};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task;

// #[repr(C)]
// pub struct MyClass {
//     value: i32,
// }

// #[no_mangle]
// pub extern "C" fn my_class_new(value: i32) -> *mut MyClass {
//     Box::into_raw(Box::new(MyClass { value }))
// }

// #[no_mangle]
// pub extern "C" fn my_class_get_value(obj: *mut MyClass) -> i32 {
//     unsafe {
//         if obj.is_null() {
//             return 0; // 或者处理错误
//         }
//         (*obj).value
//     }
// }

// #[no_mangle]
// pub extern "C" fn my_class_free(obj: *mut MyClass) {
//     if !obj.is_null() {
//         unsafe {
//             Box::from_raw(obj);
//         }
//     }
// }

use sysinfo::System;
use tokio::sync::Mutex;

lazy_static::lazy_static! {
    static ref bucky_status_scaner_mgr: Mutex<BuckyStatusScanerMgr> = Mutex::new(BuckyStatusScanerMgr {
        next_seq: 1,
        scaners: HashMap::new()
    });

    static ref buckyos_process: HashSet<&'static str> = {
        let mut set = HashSet::new();
        set.extend(["node_daemon", "scheduler", "verify_hub", "system_config", "cyfs_gateway"]);
        set
    };
}

struct BuckyStatusScanerMgr {
    next_seq: u32,
    scaners: HashMap<u32, mpsc::Sender<()>>,
}

#[repr(C)]
enum BuckyStatus {
    Running = 0,
    Stopped = 1,
    NotInstall = 2,
    Failed = 3,
}

#[repr(C)]
struct BuckyStatusScaner(u32);

type ScanStatusCallback =
    extern "C" fn(new_status: BuckyStatus, old_status: BuckyStatus, userdata: *const c_void);

#[no_mangle]
extern "C" fn bucky_status_scaner_scan(
    callback: ScanStatusCallback,
    userdata: *const c_void,
    _hwnd: *const c_void,
) -> *mut BuckyStatusScaner {
    let (sender, mut receiver) = mpsc::channel(32);
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Wrap both callback and userdata in a Send-safe wrapper
    struct CallbackWrapper {
        callback: ScanStatusCallback,
        userdata: *const c_void,
        _hwnd: *const c_void,
    }
    unsafe impl Send for CallbackWrapper {}
    unsafe impl Sync for CallbackWrapper {}
    let callback_wrapper = Arc::new(CallbackWrapper {
        callback,
        userdata,
        _hwnd,
    });

    rt.spawn(async move {
        let mut system = System::new_all();
        let mut status = BuckyStatus::Stopped;
        let mut interval = std::time::Duration::from_millis(0);
        loop {
            futures::select! {
                _ = receiver.recv().fuse() => {
                    break;
                },
                _ = tokio::time::sleep(interval).fuse() => {
                    system.refresh_all();
                    let mut exist_process = HashSet::new();
                    let mut not_exist_process = buckyos_process.clone();

                    for process in system.processes().values() {
                        let name = process.name().to_string_lossy().to_ascii_lowercase();
                        if buckyos_process.contains(name.as_str()) {
                            not_exist_process.remove(name.as_str());
                            exist_process.insert(name);
                        }
                    }

                    let old_status = status;
                    if !not_exist_process.is_empty() {
                        if !exist_process.is_empty() {
                            status = BuckyStatus::Failed;
                            interval = std::time::Duration::from_millis(500);
                            (callback_wrapper.callback)(BuckyStatus::Failed, old_status, callback_wrapper.userdata);
                        } else {
                            status = BuckyStatus::Stopped;
                            interval = std::time::Duration::from_millis(5000);
                            (callback_wrapper.callback)(BuckyStatus::Stopped, old_status, callback_wrapper.userdata);
                        }
                    } else {
                        status = BuckyStatus::Running;
                        interval = std::time::Duration::from_millis(5000);
                        (callback_wrapper.callback)(BuckyStatus::Running, old_status, callback_wrapper.userdata);
                    }
                }
            }
        }
    });

    rt.block_on(async move {
        let mut scaner_mgr = bucky_status_scaner_mgr.lock().await;
        let seq = scaner_mgr.next_seq;
        scaner_mgr.next_seq = scaner_mgr.next_seq + 1;
        scaner_mgr.scaners.insert(seq, sender);

        Box::into_raw(Box::new(BuckyStatusScaner(seq)))
    })
}

#[no_mangle]
extern "C" fn bucky_status_scaner_stop(scaner: *mut BuckyStatusScaner) {
    if !scaner.is_null() {
        let scaner = unsafe { Box::from_raw(scaner) };

        task::spawn(async move {
            let mut scaner_mgr = bucky_status_scaner_mgr.lock().await;
            let scaner = scaner_mgr.scaners.remove(&scaner.0);
            if let Some(scaner) = scaner {
                let _ = scaner.send(()).await;
            }
        });
    }
}

#[repr(C)]
struct ApplicationInfo {
    name: *const c_char,
    icon_path: *const c_char,
    home_page_url: *const c_char,
    start_cmd: *const c_char,
    stop_cmd: *const c_char,
    is_running: c_char,
}

type ListAppCallback = extern "C" fn(
    is_success: c_char,
    apps: *const ApplicationInfo,
    app_count: c_int,
    seq: c_int,
    user_data: *const c_void,
);

#[no_mangle]
extern "C" fn list_application(seq: c_int, callback: ListAppCallback, user_data: *const c_void) {
    callback(1, ptr::null(), 0, seq, user_data)
}
