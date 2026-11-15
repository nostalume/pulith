//! Owner-only protection by security descriptor on the actual file handle.

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{addr_of, null_mut};

use windows_sys::Win32::Foundation::{
    ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_FUNCTION, ERROR_NO_TOKEN, ERROR_NOT_SUPPORTED,
    ERROR_PATH_NOT_FOUND, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetLengthSid,
    GetSecurityDescriptorControl, GetTokenInformation, IsValidAcl, IsValidSid,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES,
    TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateFileW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_OPENED, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetFinalPathNameByHandleW, GetVolumeInformationByHandleW, READ_CONTROL,
    VOLUME_NAME_GUID,
};
use windows_sys::Win32::System::SystemServices::{ACCESS_ALLOWED_ACE_TYPE, FILE_PERSISTENT_ACLS};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};

use super::LocalError;

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: these allocations were returned by APIs that require LocalFree.
        unsafe { LocalFree(self.0) };
    }
}

struct Principal(Vec<usize>);

impl Principal {
    fn current() -> io::Result<Self> {
        let mut handle = null_mut();
        // SAFETY: the pseudo-handles are valid and handle is a writable out parameter.
        unsafe {
            if OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut handle) == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_NO_TOKEN as i32) {
                    return Err(error);
                }
                if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }
        // SAFETY: a successful token open returned an owned handle.
        let token = unsafe { OwnedHandle::from_raw_handle(handle) };
        let mut bytes = 0;
        // SAFETY: a zero-size query obtains the required size.
        unsafe { GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut bytes) };
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(error);
        }
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        // SAFETY: the buffer is aligned, writable, and at least bytes long.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                bytes,
                &mut bytes,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(buffer))
    }

    fn sid(&self) -> PSID {
        // SAFETY: TokenUser initializes the header and points the SID into this allocation.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }

    fn descriptor(&self, inheritance: &str) -> io::Result<LocalAllocation> {
        let mut string = null_mut();
        // SAFETY: sid is valid and string is a writable out parameter.
        if unsafe { ConvertSidToStringSidW(self.sid(), &mut string) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _string = LocalAllocation(string.cast());
        // SAFETY: the conversion returned a NUL-terminated UTF-16 string.
        let len = (0..)
            .find(|&index| unsafe { *string.add(index) } == 0)
            .unwrap();
        let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(string, len) });
        let text: Vec<u16> = format!("O:{sid}D:P(A;{inheritance};FA;;;{sid})")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let mut descriptor = null_mut();
        // SAFETY: text is NUL-terminated and descriptor is a writable out parameter.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(LocalAllocation(descriptor))
    }
}

pub(super) fn create_file(path: &Path) -> io::Result<File> {
    let descriptor = Principal::current()?.descriptor("")?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    let parent = parent.canonicalize()?;
    let path = parent.join(
        path.file_name()
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?,
    );
    let wide = super::wide_path(&path)?;
    // SAFETY: the path is terminated and attributes owns a valid descriptor for this call.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_WRITE | READ_CONTROL,
            0,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned an owned handle.
    Ok(unsafe { File::from_raw_handle(raw) })
}

pub(super) fn is_owner_only(file: &File) -> io::Result<bool> {
    let principal = Principal::current()?;
    let mut owner = null_mut();
    let mut dacl = null_mut();
    let mut raw: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: the handle is live and all requested outputs are writable.
    let error = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut raw,
        )
    };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    let _descriptor = LocalAllocation(raw);
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: raw owns the descriptor returned by GetSecurityInfo.
    if unsafe { GetSecurityDescriptorControl(raw, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: owner and dacl, when present, point into the descriptor.
    if control & SE_DACL_PROTECTED == 0
        || owner.is_null()
        || dacl.is_null()
        || unsafe { EqualSid(owner, principal.sid()) } == 0
        || unsafe { IsValidAcl(dacl) } == 0
        || unsafe { (*dacl).AceCount } != 1
    {
        return Ok(false);
    }
    let mut entry = null_mut();
    // SAFETY: the validated ACL has one entry.
    if unsafe { GetAce(dacl, 0, &mut entry) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the validated ACL contains a complete ACE header.
    let header = unsafe { &*entry.cast::<ACE_HEADER>() };
    let sid_offset = offset_of!(ACCESS_ALLOWED_ACE, SidStart);
    let ace_size = header.AceSize as usize;
    if header.AceType as u32 != ACCESS_ALLOWED_ACE_TYPE
        || header.AceFlags != 0
        || ace_size < sid_offset + 8
    {
        return Ok(false);
    }
    // SAFETY: the validated access-allowed ACE contains its SID following the mask.
    let ace = unsafe { &*entry.cast::<ACCESS_ALLOWED_ACE>() };
    let sid: PSID = addr_of!(ace.SidStart).cast_mut().cast();
    // A SID has an eight-byte header followed by the counted u32 subauthorities.
    // SAFETY: the preceding size check covers the header. The remaining checks short-circuit.
    Ok(ace.Mask == FILE_ALL_ACCESS
        && unsafe {
            IsValidSid(sid) != 0
                && GetLengthSid(sid) as usize <= ace_size - sid_offset
                && EqualSid(sid, owner) != 0
        })
}

pub(super) fn supports_private_files(file: &File) -> io::Result<bool> {
    let mut flags = 0;
    let mut fs_name = [0u16; 32];
    // SAFETY: the handle is live and the volume flags and name buffers are writable.
    if unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            &mut flags,
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let ntfs = [b'N' as u16, b'T' as u16, b'F' as u16, b'S' as u16, 0];
    if flags & FILE_PERSISTENT_ACLS == 0 || !fs_name.starts_with(&ntfs) {
        return Ok(false);
    }
    // A local volume has a volume GUID. A mapped or UNC share has none.
    // SAFETY: the handle is live and the zero-sized null buffer requests the required size.
    let size = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            null_mut(),
            0,
            FILE_NAME_OPENED | VOLUME_NAME_GUID,
        )
    };
    if size > 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(ERROR_PATH_NOT_FOUND as i32) {
        return Ok(false);
    }
    Err(error)
}

pub(super) fn check_destination(path: &Path) -> Result<(), LocalError> {
    // A private publication addresses a file, never a named stream on an existing object.
    if path
        .file_name()
        .is_none_or(|name| name.encode_wide().any(|unit| unit == b':' as u16))
    {
        return Err(LocalError::InvalidPath(path.into()));
    }
    let absolute = std::path::absolute(path)
        .map_err(|error| LocalError::io("resolve private target", path, error))?;
    for parent in absolute.ancestors().skip(1) {
        let file = match OpenOptions::new()
            .access_mode(0)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(protection_error("open private target parent", path, error)),
        };
        let metadata = file
            .metadata()
            .map_err(|error| LocalError::io("inspect private target parent", path, error))?;
        if !metadata.is_dir() {
            return Err(LocalError::io(
                "inspect private target parent",
                parent,
                io::ErrorKind::NotADirectory.into(),
            ));
        }
        return match supports_private_files(&file) {
            Ok(true) => Ok(()),
            Ok(false) => Err(LocalError::PrivatePublicationUnavailable(path.into())),
            Err(error) => Err(protection_error(
                "inspect private target volume",
                path,
                error,
            )),
        };
    }
    Err(LocalError::io(
        "find private target parent",
        path,
        io::ErrorKind::NotFound.into(),
    ))
}

pub(super) fn unsupported(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error().map(|code| code as u32),
        Some(ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED)
    )
}

pub(super) fn protection_error(action: &'static str, path: &Path, error: io::Error) -> LocalError {
    if unsupported(&error) {
        LocalError::PrivatePublicationUnavailable(path.into())
    } else {
        LocalError::io(action, path, error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    #[ignore = "requires the same NTFS fixture as windows_private_publication_two_accounts"]
    fn windows_private_stage_two_accounts() {
        let root = std::env::var_os("PULITH_PRIVATE_TEST_ROOT").expect("shared fixture directory");
        let phase = std::env::var("PULITH_PRIVATE_TEST_PHASE").expect("create, read or remove");
        let stage = Path::new(&root).join("stage");
        match phase.as_str() {
            "create" => {
                let mut file = create_file(&stage).unwrap();
                assert!(supports_private_files(&file).unwrap());
                assert!(is_owner_only(&file).unwrap());
                file.write_all(b"staged fixture").unwrap();
                file.sync_all().unwrap();
                drop(file);
                assert_eq!(std::fs::read(&stage).unwrap(), b"staged fixture");
                println!("publisher reopened the staged bytes");
            }
            "read" => {
                assert_eq!(
                    std::fs::read(Path::new(&root).join("control")).unwrap(),
                    b"public fixture"
                );
                assert_eq!(
                    std::fs::read(stage).unwrap_err().kind(),
                    io::ErrorKind::PermissionDenied
                );
                println!("reader opened the control; staged file denied read");
            }
            "remove" => std::fs::remove_file(stage).unwrap(),
            _ => panic!("use create, read or remove"),
        }
    }
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    use std::fs;
    use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    #[test]
    #[ignore = "requires the same NTFS fixture as windows_private_publication_two_accounts"]
    fn windows_private_editor_two_accounts() {
        let root = std::env::var_os("PULITH_PRIVATE_TEST_ROOT").expect("shared fixture directory");
        let phase = std::env::var("PULITH_PRIVATE_TEST_PHASE").expect("create, read or remove");
        let session = Path::new(&root).canonicalize().unwrap().join("editor");
        let document = session.join("document");
        let replacement = session.join("replacement");
        let backup = session.join("backup");
        let wide = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        match phase.as_str() {
            "create" => {
                let descriptor = Principal::current().unwrap().descriptor("OICI").unwrap();
                let attributes = SECURITY_ATTRIBUTES {
                    nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor.0,
                    bInheritHandle: 0,
                };
                // SAFETY: the path and the descriptor remain live for the call.
                assert_ne!(
                    unsafe { CreateDirectoryW(wide(&session).as_ptr(), &attributes) },
                    0
                );
                super::super::LocalPath::new(&document)
                    .unwrap()
                    .publish_private_file(b"original", super::super::TreeAdmission::Absent)
                    .unwrap();
                fs::write(&replacement, b"new").unwrap();
                // SAFETY: all three terminated path buffers remain live for the call.
                assert_ne!(
                    unsafe {
                        ReplaceFileW(
                            wide(&document).as_ptr(),
                            wide(&replacement).as_ptr(),
                            wide(&backup).as_ptr(),
                            0,
                            null_mut(),
                            null_mut(),
                        )
                    },
                    0
                );
                assert_eq!(fs::read(&document).unwrap(), b"new");
                assert_eq!(fs::read(&backup).unwrap(), b"original");
                fs::write(&replacement, b"new editor scratch").unwrap();
                assert_eq!(fs::read(&replacement).unwrap(), b"new editor scratch");
                println!("publisher read editor replacement, backup and scratch bytes");
            }
            "read" => {
                assert_eq!(
                    fs::read(Path::new(&root).join("control")).unwrap(),
                    b"public fixture"
                );
                for path in [document, replacement, backup] {
                    assert_eq!(
                        fs::read(path).unwrap_err().kind(),
                        io::ErrorKind::PermissionDenied
                    );
                }
                println!("reader opened the control; replacement, backup and scratch denied read");
            }
            "remove" => {
                for path in [document, replacement, backup] {
                    fs::remove_file(path).unwrap();
                }
                fs::remove_dir(session).unwrap();
            }
            _ => panic!("use create, read or remove"),
        }
    }
}
