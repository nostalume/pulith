use super::super::ServiceError;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    BuildExplicitAccessWithNameW, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW,
    REVOKE_ACCESS, SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, CreateWellKnownSid,
    DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetAclInformation, INHERIT_ONLY_ACE,
    LookupAccountNameW, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_APPEND_DATA, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DELETE_CHILD,
    FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA,
    WRITE_DAC, WRITE_OWNER,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;

pub(crate) struct AccessGrant {
    pub(crate) path: PathBuf,
    pub(crate) mask: u32,
    pub(crate) inheritance: u32,
}

struct Descriptor {
    owner: PSID,
    dacl: *mut ACL,
    raw: PSECURITY_DESCRIPTOR,
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.raw) };
    }
}

pub(super) fn access_plan(release: PathBuf, declaration: PathBuf) -> [AccessGrant; 2] {
    [
        AccessGrant {
            path: release,
            mask: FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            inheritance: windows_sys::Win32::Security::CONTAINER_INHERIT_ACE
                | windows_sys::Win32::Security::OBJECT_INHERIT_ACE,
        },
        AccessGrant {
            path: declaration,
            mask: FILE_GENERIC_READ,
            inheritance: 0,
        },
    ]
}

pub(super) fn has_access(grant: &AccessGrant, account: &str) -> Result<bool, ServiceError> {
    let descriptor = descriptor(&grant.path)?;
    let sid = account_sid(account)?;
    Ok(contains(
        descriptor.dacl,
        sid.as_ptr().cast_mut().cast(),
        grant,
    ))
}

pub(super) fn apply(grant: &AccessGrant, account: &str, create: bool) -> Result<(), ServiceError> {
    if !create || has_access(grant, account)? {
        return Ok(());
    }
    change(grant, account, GRANT_ACCESS)
}

pub(super) fn revoke(
    grant: &AccessGrant,
    account: &str,
    created: bool,
) -> Result<(), ServiceError> {
    if !created || !has_access(grant, account)? {
        return Ok(());
    }
    change(grant, account, REVOKE_ACCESS)
}

fn change(grant: &AccessGrant, account: &str, mode: i32) -> Result<(), ServiceError> {
    let descriptor = descriptor(&grant.path)?;
    let account = wide(OsStr::new(account));
    let mut entry = EXPLICIT_ACCESS_W::default();
    unsafe {
        BuildExplicitAccessWithNameW(
            &mut entry,
            account.as_ptr(),
            grant.mask,
            mode,
            grant.inheritance,
        );
    }
    let mut replacement = std::ptr::null_mut();
    security(
        unsafe { SetEntriesInAclW(1, &entry, descriptor.dacl, &mut replacement) },
        "build service access ACL",
    )?;
    let path = wide(grant.path.as_os_str());
    let error = unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            replacement,
            std::ptr::null(),
        )
    };
    unsafe {
        LocalFree(replacement.cast());
    }
    security(error, "set service access ACL")
}

fn descriptor(path: &Path) -> Result<Descriptor, ServiceError> {
    let path = wide(path.as_os_str());
    let mut owner = std::ptr::null_mut();
    let mut dacl = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    let error = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    security(error, "read service access ACL")?;
    Ok(Descriptor {
        owner,
        dacl,
        raw: descriptor,
    })
}

fn contains(dacl: *mut ACL, sid: PSID, grant: &AccessGrant) -> bool {
    let mut found = false;
    visit_allowed(dacl, |header, ace| {
        let current = (&ace.SidStart as *const u32).cast_mut().cast();
        found |= u32::from(header.AceFlags) & grant.inheritance == grant.inheritance
            && ace.Mask & grant.mask == grant.mask
            && unsafe { EqualSid(current, sid) != 0 };
    }) && found
}

fn account_sid(account: &str) -> Result<Vec<u8>, ServiceError> {
    let account = wide(OsStr::new(account));
    let mut sid_length = 0;
    let mut domain_length = 0;
    let mut use_kind = 0;
    unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            account.as_ptr(),
            std::ptr::null_mut(),
            &mut sid_length,
            std::ptr::null_mut(),
            &mut domain_length,
            &mut use_kind,
        );
    }
    let mut sid = vec![0_u8; sid_length as usize];
    let mut domain = vec![0_u16; domain_length as usize];
    if sid.is_empty()
        || unsafe {
            LookupAccountNameW(
                std::ptr::null(),
                account.as_ptr(),
                sid.as_mut_ptr().cast(),
                &mut sid_length,
                domain.as_mut_ptr(),
                &mut domain_length,
                &mut use_kind,
            ) == 0
        }
    {
        Err(ServiceError::os(
            "resolve service SID",
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(sid)
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

pub(crate) fn secure_leaf(path: &Path) -> Result<(), ServiceError> {
    inspect(path, LEAF_MUTATION, true)
}

pub(crate) fn secure_ancestor(path: &Path) -> Result<(), ServiceError> {
    inspect(path, CONTAINER_REPLACEMENT, true)
}

pub(crate) fn secure_input(path: &Path) -> Result<(), ServiceError> {
    inspect(path, INPUT_MUTATION, false)
}

const CONTAINER_REPLACEMENT: u32 = DELETE | FILE_DELETE_CHILD | WRITE_DAC | WRITE_OWNER;
const INPUT_MUTATION: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | DELETE
    | WRITE_DAC
    | WRITE_OWNER;
const LEAF_MUTATION: u32 = INPUT_MUTATION | FILE_DELETE_CHILD;

fn inspect(path: &Path, forbidden: u32, require_directory: bool) -> Result<(), ServiceError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| ServiceError::operation("inspect privileged root", error))?;
    if (require_directory && !metadata.file_type().is_dir())
        || (!require_directory && !metadata.file_type().is_dir() && !metadata.file_type().is_file())
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(ServiceError::invalid(
            "service root contains a reparse point",
        ));
    }
    let descriptor = descriptor(path)?;
    if trusted(descriptor.owner) && protected_dacl(descriptor.dacl, forbidden) {
        Ok(())
    } else {
        Err(ServiceError::invalid(
            "service root owner and write authority must be limited to SYSTEM or Administrators",
        ))
    }
}

fn protected_dacl(dacl: *mut ACL, forbidden: u32) -> bool {
    let mut protected = true;
    visit_allowed(dacl, |header, ace| {
        if u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0 {
            return;
        }
        let sid = (&ace.SidStart as *const u32).cast_mut().cast();
        if ace.Mask & forbidden != 0 && !trusted(sid) {
            protected = false;
        }
    }) && protected
}

fn visit_allowed(
    dacl: *mut ACL,
    mut visit: impl FnMut(&windows_sys::Win32::Security::ACE_HEADER, &ACCESS_ALLOWED_ACE),
) -> bool {
    if dacl.is_null() {
        return false;
    }
    let mut facts = ACL_SIZE_INFORMATION::default();
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut facts as *mut ACL_SIZE_INFORMATION).cast(),
            std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return false;
    }
    for index in 0..facts.AceCount {
        let mut raw = std::ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut raw) } == 0 {
            return false;
        }
        let header = unsafe { &*(raw as *const windows_sys::Win32::Security::ACE_HEADER) };
        if header.AceType == ACCESS_ALLOWED_ACE_TYPE as u8 {
            visit(header, unsafe { &*(raw as *const ACCESS_ALLOWED_ACE) });
        }
    }
    true
}

fn trusted(sid: PSID) -> bool {
    known(sid, WinLocalSystemSid)
        || known(sid, WinBuiltinAdministratorsSid)
        || named(sid, "NT SERVICE\\TrustedInstaller")
}

fn named(sid: PSID, account: &str) -> bool {
    account_sid(account)
        .is_ok_and(|mut account| unsafe { EqualSid(sid, account.as_mut_ptr().cast()) != 0 })
}

fn known(owner: PSID, kind: i32) -> bool {
    let mut bytes = [0_u8; 68];
    let mut length = bytes.len() as u32;
    unsafe {
        CreateWellKnownSid(
            kind,
            std::ptr::null_mut(),
            bytes.as_mut_ptr().cast(),
            &mut length,
        ) != 0
            && EqualSid(owner, bytes.as_mut_ptr().cast()) != 0
    }
}

fn security(result: u32, action: &'static str) -> Result<(), ServiceError> {
    if result == 0 {
        Ok(())
    } else {
        Err(ServiceError::os(
            action,
            std::io::Error::from_raw_os_error(result as i32),
        ))
    }
}
