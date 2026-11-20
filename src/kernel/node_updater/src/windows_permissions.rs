//! Create private staging directories atomically and preserve an existing store's ACL.
use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_SUCCESS, HANDLE},
    Security::{
        AdjustTokenPrivileges,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
            SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
        },
        EqualSid, GetAce, GetSecurityDescriptorControl, GetTokenInformation, IsWellKnownSid,
        LookupPrivilegeValueW, SetFileSecurityW, SetSecurityDescriptorControl, TokenUser,
        WinBuiltinAdministratorsSid, WinCreatorOwnerRightsSid, WinLocalSystemSid,
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
        SE_DACL_AUTO_INHERITED, SE_DACL_AUTO_INHERIT_REQ, SE_DACL_PROTECTED, SE_PRIVILEGE_ENABLED,
        SE_RESTORE_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER,
    },
    Storage::FileSystem::CreateDirectoryW,
    System::{
        SystemServices::ACCESS_ALLOWED_ACE_TYPE,
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

pub fn private_dir(path: &Path) -> io::Result<()> {
    let acl = wide(OsStr::new(
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)",
    ));
    let mut descriptor = ptr::null_mut();
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            acl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attrs = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = CreateDirectoryW(wide(path.as_os_str()).as_ptr(), &attrs);
        let error = if result == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        LocalFree(descriptor);
        error.map_or(Ok(()), Err)
    }
}

struct Security {
    descriptor: PSECURITY_DESCRIPTOR,
    owner: PSID,
    dacl: *mut ACL,
    protected: bool,
    auto_inherited: bool,
}

impl Drop for Security {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

impl Security {
    fn read(path: &Path) -> io::Result<Self> {
        let mut info = Self {
            descriptor: ptr::null_mut(),
            owner: ptr::null_mut(),
            dacl: ptr::null_mut(),
            protected: false,
            auto_inherited: false,
        };
        unsafe {
            let code = GetNamedSecurityInfoW(
                wide(path.as_os_str()).as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut info.owner,
                ptr::null_mut(),
                &mut info.dacl,
                ptr::null_mut(),
                &mut info.descriptor,
            );
            if code != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(code as i32));
            }
            let (mut control, mut rev) = (0, 0);
            if GetSecurityDescriptorControl(info.descriptor, &mut control, &mut rev) == 0 {
                return Err(io::Error::last_os_error());
            }
            info.auto_inherited = control & SE_DACL_AUTO_INHERITED != 0;
            info.protected = control & SE_DACL_PROTECTED != 0;
        }
        Ok(info)
    }
}

struct Token(HANDLE);
impl Token {
    fn open(rights: u32) -> io::Result<Self> {
        let mut token = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), rights, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(token))
    }
}
impl Drop for Token {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn check_private_state(path: &Path) -> io::Result<()> {
    let info = Security::read(path)?;
    let token = Token::open(TOKEN_QUERY)?;
    let denied = || {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "migration state directory has unsafe ownership or access rules",
        )
    };
    unsafe {
        let mut size = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
        if size == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut data = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            data.as_mut_ptr().cast(),
            size,
            &mut size,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let caller = (*data.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let trusted = |sid: PSID| {
            !sid.is_null()
                && (EqualSid(sid, caller) != 0
                    || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                    || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0)
        };
        if !trusted(info.owner) || !info.protected || info.dacl.is_null() {
            return Err(denied());
        }
        for i in 0..(*info.dacl).AceCount {
            let mut ace = ptr::null_mut();
            if GetAce(info.dacl, i.into(), &mut ace) == 0 {
                return Err(io::Error::last_os_error());
            }
            if u32::from((*ace.cast::<ACE_HEADER>()).AceType) != ACCESS_ALLOWED_ACE_TYPE {
                return Err(denied());
            }
            let sid = (&raw mut (*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart).cast();
            if !trusted(sid) && IsWellKnownSid(sid, WinCreatorOwnerRightsSid) == 0 {
                return Err(denied());
            }
        }
    }
    Ok(())
}

fn set_owner(path: &[u16], owner: PSID) -> io::Result<()> {
    // An elevated administrator needs SeRestorePrivilege to restore another
    // user's ownership. Restore the token's original privilege immediately.
    let token = Token::open(TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES)?;
    unsafe {
        let mut privilege = TOKEN_PRIVILEGES::default();
        privilege.PrivilegeCount = 1;
        let entry = &mut privilege.Privileges[0];
        entry.Attributes = SE_PRIVILEGE_ENABLED;
        if LookupPrivilegeValueW(ptr::null(), SE_RESTORE_NAME, &mut entry.Luid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut prior = TOKEN_PRIVILEGES::default();
        let mut size = 0;
        if AdjustTokenPrivileges(
            token.0,
            0,
            &privilege,
            size_of::<TOKEN_PRIVILEGES>() as u32,
            &mut prior,
            &mut size,
        ) == 0
            || GetLastError() != ERROR_SUCCESS
        {
            return Err(io::Error::last_os_error());
        }
        let code = SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            owner,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
        );
        if AdjustTokenPrivileges(token.0, 0, &prior, 0, ptr::null_mut(), ptr::null_mut()) == 0 {
            return Err(io::Error::last_os_error());
        }
        if code != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(code as i32));
        }
    }
    Ok(())
}

pub fn copy_dacl(from: &Path, to: &Path) -> io::Result<()> {
    let source = Security::read(from)?;
    let destination = Security::read(to)?;
    let to = wide(to.as_os_str());
    unsafe {
        if EqualSid(source.owner, destination.owner) == 0 {
            set_owner(&to, source.owner)?;
        }
        if source.protected {
            let code = SetNamedSecurityInfoW(
                to.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                source.dacl,
                ptr::null(),
            );
            if code != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(code as i32));
            }
        } else {
            // SetFileSecurity requires an explicit inheritance request to retain the
            // AUTO_INHERITED bit while copying the original inherited ACEs.
            if source.auto_inherited
                && SetSecurityDescriptorControl(
                    source.descriptor,
                    SE_DACL_AUTO_INHERIT_REQ,
                    SE_DACL_AUTO_INHERIT_REQ,
                ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            if SetFileSecurityW(to.as_ptr(), DACL_SECURITY_INFORMATION, source.descriptor) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_auto_inheritance_through_private_staging() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("live");
        let stage = dir.path().join("private");
        private_dir(&live).unwrap();
        private_dir(&stage).unwrap();
        let source = live.join("source");
        let target = stage.join("new");
        std::fs::write(&source, "old").unwrap();
        std::fs::write(&target, "new").unwrap();
        // Opt the source into Windows automatic inheritance explicitly. The
        // default model of a temp directory varies with its parent's ACL.
        let initial = Security::read(&source).unwrap();
        unsafe {
            assert_eq!(
                SetNamedSecurityInfoW(
                    wide(source.as_os_str()).as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION
                        | windows_sys::Win32::Security::UNPROTECTED_DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    initial.dacl,
                    ptr::null(),
                ),
                ERROR_SUCCESS
            );
        }
        let original = Security::read(&source).unwrap();
        assert!(original.auto_inherited && !original.protected);
        copy_dacl(&source, &target).unwrap();
        let copied = Security::read(&target).unwrap();
        assert!(copied.auto_inherited && !copied.protected);
    }
}
