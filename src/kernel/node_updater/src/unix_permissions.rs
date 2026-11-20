use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

pub fn owner(source: &Path, destination: &Path) -> io::Result<()> {
    let source = fs::symlink_metadata(source)?;
    let current = fs::symlink_metadata(destination)?;
    if source.uid() != current.uid() || source.gid() != current.gid() {
        std::os::unix::fs::chown(destination, Some(source.uid()), Some(source.gid()))?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn access_metadata(source: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{
        getxattr, getxattr_query, listxattr, listxattr_query, removexattr, setxattr, XattrFlags,
    };
    use rustix::io::Errno;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let size = match listxattr_query(source) {
        Err(Errno::NOTSUP) => 0,
        other => other?,
    };
    let mut names = vec![0; size];
    if size != 0 {
        let used = listxattr(source, &mut names[..])?;
        names.truncate(used);
    }
    for name in names
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .map(OsStr::from_bytes)
    {
        let size = getxattr_query(source, name)?;
        let mut value = vec![0; size];
        if size != 0 {
            let used = getxattr(source, name, &mut value[..])?;
            value.truncate(used);
        }
        let mut current = vec![0; value.len().max(1)];
        if let Ok(used) = getxattr(destination, name, &mut current[..]) {
            if current[..used] == value {
                continue;
            }
        }
        setxattr(destination, name, &value, XattrFlags::empty())?;
    }
    // A staging parent can supply a default ACL even when the original did not have one.
    for acl in [
        b"system.posix_acl_access".as_slice(),
        b"system.posix_acl_default",
    ] {
        if !names.split(|b| *b == 0).any(|n| n == acl) {
            match removexattr(destination, OsStr::from_bytes(acl)) {
                Ok(()) | Err(Errno::NODATA | Errno::NOTSUP) => (),
                Err(err) => return Err(err.into()),
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn access_metadata(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    let flags = libc::COPYFILE_ACL | libc::COPYFILE_XATTR | libc::COPYFILE_NOFOLLOW;
    if unsafe {
        libc::copyfile(
            source.as_ptr(),
            destination.as_ptr(),
            std::ptr::null_mut(),
            flags,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
