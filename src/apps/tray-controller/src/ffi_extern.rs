use futures::FutureExt;
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::sync::Arc;
use std::{iter, ptr};
use tokio::sync::mpsc;
use tokio::task;

use buckyos_kit::*;
use serde::Deserialize;
use std::fs;

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

    static ref node_infomation: Arc<Mutex<Option<NodeInfomationObj>>> = Arc::new(Mutex::new(None));
}

struct BuckyStatusScanerMgr {
    next_seq: u32,
    scaners: HashMap<u32, mpsc::Sender<()>>,
}

#[repr(C)]
#[derive(PartialEq, Eq, Copy, Clone)]
enum BuckyStatus {
    Running = 0,
    Stopped = 1,
    NotActive = 2,
    NotInstall = 3,
    Failed = 4,
}

#[repr(C)]
struct BuckyStatusScaner(u32);

struct NodeInfomationObj {
    node_id: String,
    home_page_url: String,
}

#[repr(C)]
struct NodeInfomation {
    node_id: *mut c_char,
    home_page_url: *mut c_char,
}

unsafe impl Send for NodeInfomation {}
unsafe impl Sync for NodeInfomation {}

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
                    let old_status = status;

                    unsafe {
                        let info = get_node_info();
                        if info.is_null() || (*info).node_id.is_null() {
                            status = BuckyStatus::NotActive;
                            interval = std::time::Duration::from_millis(5000);
                        }
                        free_node_info(info);
                    }

                    if status != BuckyStatus::NotActive {

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

                        if !not_exist_process.is_empty() {
                            if !exist_process.is_empty() {
                                status = BuckyStatus::Failed;
                                interval = std::time::Duration::from_millis(500);
                            } else {
                                status = BuckyStatus::Stopped;
                                interval = std::time::Duration::from_millis(5000);
                            }
                        } else {
                            status = BuckyStatus::Running;
                            interval = std::time::Duration::from_millis(5000);
                        }
                    }

                    if status != old_status {
                        (callback_wrapper.callback)(status, old_status, callback_wrapper.userdata);
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

//NodeIdentity from ood active progress
#[derive(Deserialize, Debug)]
struct NodeIdentityConfig {
    zone_name: String, // $name.buckyos.org or did:ens:$name
    // owner_public_key: jsonwebtoken::jwk::Jwk, //owner is zone_owner
    owner_name: String,     //owner's name
    device_doc_jwt: String, //device document,jwt string,siged by owner
    zone_nonce: String,     // random string, is default password of some service
                            //device_private_key: ,storage in partical file
}

type NodeId = String;
type StrError = String;

fn list_nodes() -> Result<HashMap<NodeId, NodeIdentityConfig>, StrError> {
    let etc_dir = get_buckyos_system_etc_dir();

    let mut nodes = HashMap::new();

    for entry in fs::read_dir(etc_dir).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let file_path = entry.path();

        if file_path.is_file() {
            if let Some(file_name) = file_path.file_name().and_then(|name| name.to_str()) {
                if let Some(node_id) = file_name.strip_suffix("_identity.toml") {
                    let contents = std::fs::read_to_string(file_path.as_path())
                        .map_err(|err| err.to_string())?;

                    let config: NodeIdentityConfig =
                        toml::from_str(&contents).map_err(|err| err.to_string())?;

                    nodes.insert(node_id.to_string(), config);
                }
            }
        }
    }

    Ok(nodes)
}

#[no_mangle]
extern "C" fn get_node_info() -> *mut NodeInfomation {
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async move {
        let mut info = node_infomation.lock().await;
        let is_actived = info.is_some();
        if !is_actived {
            if let Ok(nodes) = list_nodes() {
                if let Some((node_id, cfg)) = nodes.iter().next() {
                    *info = Some(NodeInfomationObj {
                        node_id: node_id.to_owned(),
                        home_page_url: format!("http://{}.web3.buckyos.io", cfg.owner_name),
                    })
                }
            }
        }

        let is_actived = info.is_some();
        let c_info = if is_actived {
            let info = info.as_ref().unwrap();
            NodeInfomation {
                node_id: CString::new(info.node_id.clone())
                    .expect("no memory for c_node_id")
                    .into_raw(),
                home_page_url: CString::new(info.home_page_url.clone())
                    .expect("no memory for c_home_page_url")
                    .into_raw(),
            }
        } else {
            NodeInfomation {
                node_id: std::ptr::null_mut(),
                home_page_url: CString::new("http://127.0.0.1:3180/index.html")
                    .expect("no memory for c_home_page_url")
                    .into_raw(),
            }
        };

        Box::into_raw(Box::new(c_info))
    })
}

#[no_mangle]
extern "C" fn free_node_info(info: *mut NodeInfomation) {
    if !info.is_null() {
        unsafe {
            let info = Box::from_raw(info);
            if !info.node_id.is_null() {
                let _ = CString::from_raw(info.node_id);
            }
            if !info.home_page_url.is_null() {
                let _ = CString::from_raw(info.home_page_url);
            }
        }
    }
}
