
use std::ffi::{c_void, CString, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{GetLastError, LocalFree, ERROR_INVALID_LEVEL, ERROR_MORE_DATA, ERROR_SUCCESS, GENERIC_ALL, GENERIC_READ, GENERIC_WRITE, HLOCAL};
use windows::Win32::NetworkManagement::NetManagement::{NERR_Success, NetApiBufferFree, NetUserAdd, NetUserChangePassword, NetUserGetInfo, ERRLOG2_BASE, MAX_PREFERRED_LENGTH, UF_SCRIPT, USER_INFO_1, USER_PRIV_USER};
use windows::Win32::Security::{InitializeSecurityDescriptor, IsValidSecurityDescriptor, LookupAccountNameW, SetSecurityDescriptorDacl, ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID, SECURITY_DESCRIPTOR_RELATIVE, SID, SID_NAME_USE, SUB_CONTAINERS_AND_OBJECTS_INHERIT};
use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, MULTIPLE_TRUSTEE_OPERATION, NO_MULTIPLE_TRUSTEE, SET_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_NAME, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W};
use windows::Win32::Storage::FileSystem::{NetShareAdd, NetShareDel, NetShareEnum, NetShareGetInfo, NetShareSetInfo, ACCESS_READ, SHARE_INFO_2, SHARE_INFO_502, SHARE_INFO_PERMISSIONS, SHARE_NETNAME_PARMNUM, SHARE_TYPE, SHARE_TYPE_PARMNUM, STYPE_DEVICE, STYPE_DISKTREE, STYPE_IPC};
use windows::Win32::System::SystemServices::SECURITY_DESCRIPTOR_REVISION;
use crate::error::{into_smb_err, smb_err, SmbErrorCode, SmbResult};
use crate::samba::{SmbItem, SmbUserItem};

fn add_user(user_name: &str, passwd: &str) -> SmbResult<()> {
    let mut name = OsString::from(format!("{}\0", user_name)).encode_wide().collect::<Vec<_>>();
    let mut passwd = OsString::from(format!("{}\0", passwd)).encode_wide().collect::<Vec<_>>();
    let ui = USER_INFO_1 {
        usri1_name: PWSTR::from_raw(name.as_mut_ptr()),
        usri1_password: PWSTR::from_raw(passwd.as_mut_ptr()),
        usri1_password_age: 0,
        usri1_priv: USER_PRIV_USER,
        usri1_home_dir: PWSTR::null(),
        usri1_comment: PWSTR::null(),
        usri1_flags: UF_SCRIPT,
        usri1_script_path: PWSTR::null(),
    };
    let mut param_err: u32 = 0;
    unsafe {
        let ret = NetUserAdd(PWSTR::null(), 1, (&ui as *const USER_INFO_1) as *const u8, Some(&mut param_err));
        if ret != 0 {
            Err(smb_err!(SmbErrorCode::Failed, "NetUserAdd failed: {}, param_err {}", ret, param_err))
        } else {
            Ok(())
        }
    }
}

pub fn exist_system_user(user_name: &str) -> bool {
    let mut name = OsString::from(format!("{}\0", user_name)).encode_wide().collect::<Vec<_>>();
    let mut buf: *mut u8 = std::ptr::null_mut();
    unsafe {
        let ret = NetUserGetInfo(PWSTR::null(), PWSTR::from_raw(name.as_mut_ptr()), 1, &mut buf);
        if !buf.is_null() {
            NetApiBufferFree(Some(buf as *const c_void));
        }
        if ret != NERR_Success {
            false
        } else {
            true
        }
    }
}

fn change_user_password(user_name: &str, oldpassword: &str, newpassword: &str) -> SmbResult<()> {
    let mut name = OsString::from(format!("{}\0", user_name)).encode_wide().collect::<Vec<_>>();
    let mut oldpasswd = OsString::from(format!("{}\0", oldpassword)).encode_wide().collect::<Vec<_>>();
    let mut newpasswd = OsString::from(format!("{}\0", newpassword)).encode_wide().collect::<Vec<_>>();
    unsafe {
        let ret = NetUserChangePassword(PWSTR::null(),
                                        PWSTR::from_raw(name.as_mut_ptr()),
                                        PWSTR::from_raw(oldpasswd.as_mut_ptr()),
                                        PWSTR::from_raw(newpasswd.as_mut_ptr()));
        if ret != NERR_Success {
            Err(smb_err!(SmbErrorCode::Failed, "NetUserChangePassword failed: {}", ret))
        } else {
            Ok(())
        }
    }
}

fn add_share(share_name: &str, share_path: &str, share_remark: &str, allow_users: Vec<String>) -> SmbResult<()> {
    let mut netname = OsString::from(format!("{}\0", share_name)).encode_wide().collect::<Vec<_>>();
    let mut path = OsString::from(format!("{}\0", share_path)).encode_wide().collect::<Vec<_>>();
    let mut remark = OsString::from(format!("{}\0", share_remark)).encode_wide().collect::<Vec<_>>();
    unsafe {

        let mut acl_list = Vec::new();
        for user in allow_users.iter() {
            let mut name = OsString::from(format!("{}\0", user)).encode_wide().collect::<Vec<_>>();
            let mut sid_size: u32 = 0;
            let mut domain_size: u32 = 0;
            let mut sid_type = SID_NAME_USE::default();
            let _ = LookupAccountNameW(PWSTR::null(), PWSTR::from_raw(name.as_mut_ptr()), PSID::default(), &mut sid_size, PWSTR::null(), &mut domain_size, &mut sid_type);
            let mut sid = vec![0u16; sid_size as usize];
            let mut domain = vec![0u16; domain_size as usize];
            LookupAccountNameW(PWSTR::null(), PWSTR::from_raw(name.as_mut_ptr()), PSID(sid.as_mut_ptr() as *mut c_void), &mut sid_size, PWSTR::from_raw(domain.as_mut_ptr()), &mut domain_size, &mut sid_type)
                .map_err(into_smb_err!(SmbErrorCode::Failed, "LookupAccountNameW failed"))?;
            acl_list.push(EXPLICIT_ACCESS_W {
                grfAccessPermissions: GENERIC_ALL.0,
                grfAccessMode: SET_ACCESS,
                grfInheritance: NO_INHERITANCE,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: PWSTR::from_raw(sid.as_mut_ptr()),
                },
            });
        }

        let mut acl: *mut ACL = std::ptr::null_mut();
        let err = SetEntriesInAclW(Some(acl_list.as_slice()), None, &mut acl);
        if err != ERROR_SUCCESS {
            return Err(smb_err!(SmbErrorCode::Failed, "SetEntriesInAclW failed: {}", err.0));
        }

        let mut buf = [0u8; 20];
        let sd = PSECURITY_DESCRIPTOR(buf.as_mut_ptr() as *mut c_void);
        InitializeSecurityDescriptor(sd, SECURITY_DESCRIPTOR_REVISION).map_err(into_smb_err!(SmbErrorCode::Failed, "InitializeSecurityDescriptor failed"))?;
        SetSecurityDescriptorDacl(sd, true, Some(acl), false).map_err(into_smb_err!(SmbErrorCode::Failed, "SetSecurityDescriptorDacl failed"))?;

        let p = SHARE_INFO_502 {
            shi502_reserved: 0,
            shi502_netname: PWSTR::from_raw(netname.as_mut_ptr()),
            shi502_type: STYPE_DISKTREE,
            shi502_remark: PWSTR::from_raw(remark.as_mut_ptr()),
            shi502_permissions: ACCESS_READ,
            shi502_max_uses: u32::MAX,
            shi502_current_uses: 0,
            shi502_path: PWSTR::from_raw(path.as_mut_ptr()),
            shi502_passwd: PWSTR::null(),
            shi502_security_descriptor: sd,
        };
        let mut param_err: u32 = 0;
        let ret = NetShareAdd(PWSTR::null(),
                              502,
                              (&p as *const SHARE_INFO_502) as *const u8,
                              Some(&mut param_err));
        LocalFree(HLOCAL(acl as *mut c_void));
        if ret != 0 {
            return Err(smb_err!(SmbErrorCode::Failed, "NetShareAdd failed: {}, param_err {}", ret, param_err))
        }
        Ok(())
    }
}

fn set_share_allow_users(share_name: &str, allow_users: Vec<String>) -> SmbResult<()> {
    unsafe {
        let mut netname = OsString::from(format!("{}\0", share_name)).encode_wide().collect::<Vec<_>>();
        let mut buf: *mut u8 = std::ptr::null_mut();
        let ret = NetShareGetInfo(PWSTR::null(), PWSTR::from_raw(netname.as_mut_ptr()), 502, &mut buf);
        if ret != 0 {
            return Err(smb_err!(SmbErrorCode::Failed, "NetShareGetInfo failed: {}", ret));
        }
        let mut p = buf as *const SHARE_INFO_502;
        let share = p.read();

        let mut acl_list = Vec::new();
        for user in allow_users.iter() {
            let mut name = OsString::from(format!("{}\0", user)).encode_wide().collect::<Vec<_>>();
            let mut sid_size: u32 = 0;
            let mut domain_size: u32 = 0;
            let mut sid_type = SID_NAME_USE::default();
            let _ = LookupAccountNameW(PWSTR::null(), PWSTR::from_raw(name.as_mut_ptr()), PSID::default(), &mut sid_size, PWSTR::null(), &mut domain_size, &mut sid_type);
            let mut sid = vec![0u16; sid_size as usize];
            let mut domain = vec![0u16; domain_size as usize];
            LookupAccountNameW(PWSTR::null(), PWSTR::from_raw(name.as_mut_ptr()), PSID(sid.as_mut_ptr() as *mut c_void), &mut sid_size, PWSTR::from_raw(domain.as_mut_ptr()), &mut domain_size, &mut sid_type)
                .map_err(into_smb_err!(SmbErrorCode::Failed, "LookupAccountNameW failed"))?;
            acl_list.push(EXPLICIT_ACCESS_W {
                grfAccessPermissions: GENERIC_ALL.0,
                grfAccessMode: SET_ACCESS,
                grfInheritance: NO_INHERITANCE,
                Trustee: TRUSTEE_W {
                    pMultipleTrustee: std::ptr::null_mut(),
                    MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
                    TrusteeForm: TRUSTEE_IS_SID,
                    TrusteeType: TRUSTEE_IS_USER,
                    ptstrName: PWSTR::from_raw(sid.as_mut_ptr()),
                },
            });
        }
        let mut acl: *mut ACL = std::ptr::null_mut();
        let err = SetEntriesInAclW(Some(acl_list.as_slice()), None, &mut acl);
        if err != ERROR_SUCCESS {
            return Err(smb_err!(SmbErrorCode::Failed, "SetEntriesInAclW failed: {}", err.0));
        }
        let sd = if IsValidSecurityDescriptor(share.shi502_security_descriptor).0 != 0 {
            share.shi502_security_descriptor.clone()
        } else {
            let mut buf = [0u8; 20];
            let sd = PSECURITY_DESCRIPTOR(buf.as_mut_ptr() as *mut c_void);
            InitializeSecurityDescriptor(sd, SECURITY_DESCRIPTOR_REVISION).map_err(into_smb_err!(SmbErrorCode::Failed, "InitializeSecurityDescriptor failed"))?;
            sd
        };
        SetSecurityDescriptorDacl(sd.clone(), true, Some(acl), false)
            .map_err(into_smb_err!(SmbErrorCode::Failed, "SetSecurityDescriptorDacl failed"))?;
        LocalFree(HLOCAL(acl as *mut c_void));
        let mut param_err = 0;
        NetShareSetInfo(PWSTR::null(), PWSTR::from_raw(netname.as_mut_ptr()), 502, buf as *const u8, Some(&mut param_err));

        NetApiBufferFree(Some(buf as *const c_void));
        Ok(())
    }
}

fn is_share(share_name: &str, path: &str) -> SmbResult<bool> {
    let mut buf: *mut u8 = std::ptr::null_mut();
    let mut er: u32 = 0;
    let mut tr: u32 = 0;
    let mut resume: u32 = 0;
    unsafe {
        loop {
            let ret = NetShareEnum(PWSTR::null(), 502, &mut buf, MAX_PREFERRED_LENGTH, &mut er, &mut tr, Some(&mut resume));
            if ret == ERROR_SUCCESS.0 || ret == ERROR_MORE_DATA.0 {
                let mut p = buf as *const SHARE_INFO_502;
                for _ in 0..tr {
                    let share = p.read();
                    let shi502_path = OsString::from_wide(share.shi502_path.as_wide()).to_string_lossy().to_string();
                    let shi502_netname = OsString::from_wide(share.shi502_netname.as_wide()).to_string_lossy().to_string();
                    if shi502_path.as_str() == path && shi502_netname.as_str() == share_name {
                        return Ok(true);
                    }
                    p = p.add(1);
                }
                NetApiBufferFree(Some(buf as *const c_void));
                if ret == ERROR_SUCCESS.0 {
                    return Ok(false);
                }
            } else {
                return Err(smb_err!(SmbErrorCode::Failed, "NetShareEnum failed: {}", ret));
            }
        }
    }
}

fn delete_share(share_name: &str) -> SmbResult<()> {
    let mut name = OsString::from(format!("{}\0", share_name)).encode_wide().collect::<Vec<_>>();
    unsafe {
        let ret = NetShareDel(PWSTR::null(), PWSTR::from_raw(name.as_mut_ptr()), 0);
        if ret != 0 {
            Err(smb_err!(SmbErrorCode::Failed, "NetShareDel failed: {}", ret))
        } else {
            Ok(())
        }
    }
}

pub async fn update_samba_conf(_remove_users: Vec<SmbUserItem>, new_all_users: Vec<SmbUserItem>, _remove_list: Vec<SmbItem>, new_samba_list: Vec<SmbItem>) -> SmbResult<()> {
    Ok(())
}
