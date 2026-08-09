use super::{
    Binding, Boot, ManagerObservation, NormalizedDecl, Registration, Runtime, ServiceError,
    ServiceRoot,
};
use serde::{Deserialize, Serialize};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Storage::FileSystem::DELETE;

#[path = "windows/security.rs"]
mod security;
pub(super) use security::{secure_ancestor, secure_input, secure_leaf};

pub(super) fn access_plan(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> [security::AccessGrant; 2] {
    security::access_plan(binding.release.clone(), root.declaration(declaration))
}

use windows_sys::Win32::System::Services::*;

struct ServiceHandle(SC_HANDLE);

pub(super) const ACCOUNT: &str = "NT AUTHORITY\\LocalService";
pub(super) const SID_TYPE: u32 = 3;
pub(super) const REQUIRED_PRIVILEGES: &[&str] = &[];
pub(super) const CREATED_ACCESS: u32 =
    SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS;

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}

pub fn observe(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
) -> Result<ManagerObservation, ServiceError> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let name = wide(declaration.id.as_str());
    let raw = unsafe {
        OpenServiceW(
            manager.0,
            name.as_ptr(),
            SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS,
        )
    };
    if raw.is_null() {
        return match std::io::Error::last_os_error().raw_os_error() {
            Some(1060) => Ok(ManagerObservation {
                registration: Registration::Missing,
                boot: Boot::Disabled,
                runtime: Runtime::Stopped,
            }),
            Some(1072) => Ok(ManagerObservation {
                registration: Registration::Removing,
                boot: Boot::Conflict,
                runtime: Runtime::Stopping,
            }),
            _ => Err(last("open service")),
        };
    }
    let service = ServiceHandle(raw);
    let (command, start_type, account) = query_config(&service)?;
    let binding = binding_from_command(root, declaration, &command).ok();
    let binding_exact = binding
        .as_ref()
        .is_some_and(|binding| command == render_definition(root, declaration, binding));
    let registration = if !binding_exact || !account.eq_ignore_ascii_case(ACCOUNT) {
        Registration::Conflict
    } else if query_security(&service)?
        && access_exact(root, declaration, binding.as_ref().unwrap())?
    {
        Registration::Exact
    } else {
        Registration::Broken
    };
    let boot = match start_type {
        SERVICE_AUTO_START => Boot::Enabled,
        SERVICE_DEMAND_START => Boot::Disabled,
        _ => Boot::Conflict,
    };
    Ok(ManagerObservation {
        registration,
        boot,
        runtime: query_runtime(&service)?,
    })
}

pub fn binding(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<Binding, ServiceError> {
    let service = open(declaration, SERVICE_QUERY_CONFIG)?;
    let (command, _, _) = query_config(&service)?;
    let binding = binding_from_command(root, declaration, &command)?;
    if command == render_definition(root, declaration, &binding) {
        Ok(binding)
    } else {
        Err(ServiceError::invalid("service binding conflicts"))
    }
}

pub fn install(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> Result<(), ServiceError> {
    let manager = manager(SC_MANAGER_CREATE_SERVICE)?;
    let name = wide(declaration.id.as_str());
    let command = wide(&render_definition(root, declaration, binding));
    let account = wide(ACCOUNT);
    let service = unsafe {
        CreateServiceW(
            manager.0,
            name.as_ptr(),
            name.as_ptr(),
            CREATED_ACCESS,
            SERVICE_WIN32_OWN_PROCESS,
            SERVICE_DEMAND_START,
            SERVICE_ERROR_NORMAL,
            command.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null(),
            account.as_ptr(),
            std::ptr::null(),
        )
    };
    if service.is_null() {
        Err(last("create service"))
    } else {
        let service = ServiceHandle(service);
        configure_security(&service)?;
        apply_access(root, declaration, binding)
    }
}

pub fn repair(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    let service = open(
        declaration,
        SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS,
    )?;
    let (command, _, account) = query_config(&service)?;
    let binding = binding_from_command(root, declaration, &command)?;
    if command != render_definition(root, declaration, &binding)
        || !account.eq_ignore_ascii_case(ACCOUNT)
    {
        return Err(ServiceError::invalid("service registration conflicts"));
    }
    configure_security(&service)?;
    apply_access(root, declaration, &binding)
}

fn configure_security(service: &ServiceHandle) -> Result<(), ServiceError> {
    let sid = SERVICE_SID_INFO {
        dwServiceSidType: SID_TYPE,
    };
    set_config2(service, SERVICE_CONFIG_SERVICE_SID_INFO, &sid)?;
    let mut empty = [0_u16; 2];
    let privileges = SERVICE_REQUIRED_PRIVILEGES_INFOW {
        pmszRequiredPrivileges: empty.as_mut_ptr(),
    };
    set_config2(
        service,
        SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO,
        &privileges,
    )
}

fn set_config2<T>(service: &ServiceHandle, kind: u32, value: &T) -> Result<(), ServiceError> {
    win32(
        unsafe { ChangeServiceConfig2W(service.0, kind, (value as *const T).cast()) },
        "configure service security",
    )
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AccessReceipt {
    schema: u8,
    release: String,
    created: [bool; 2],
}

fn apply_access(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> Result<(), ServiceError> {
    let account = service_account(declaration);
    let plan = access_plan(root, declaration, binding);
    let receipt_path = access_receipt(root, declaration);
    let receipt = match read_receipt(root, declaration)? {
        Some(receipt) => {
            if receipt.release != binding.relative(root).to_string_lossy() {
                revoke_receipt(root, declaration, &receipt)?;
                std::fs::remove_file(&receipt_path).map_err(|error| {
                    ServiceError::invalid(format!("remove stale access receipt: {error}"))
                })?;
                return apply_access(root, declaration, binding);
            }
            receipt
        }
        None => {
            let receipt = AccessReceipt {
                schema: 1,
                release: binding.relative(root).to_string_lossy().into_owned(),
                created: [
                    !security::has_access(&plan[0], &account)?,
                    !security::has_access(&plan[1], &account)?,
                ],
            };
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&receipt_path)
                .map_err(|error| {
                    ServiceError::invalid(format!("create access receipt: {error}"))
                })?;
            file.write_all(
                toml::to_string(&receipt)
                    .map_err(|error| {
                        ServiceError::invalid(format!("render access receipt: {error}"))
                    })?
                    .as_bytes(),
            )
            .and_then(|()| file.sync_all())
            .map_err(|error| ServiceError::invalid(format!("persist access receipt: {error}")))?;
            receipt
        }
    };
    for (grant, created) in plan.iter().zip(receipt.created) {
        security::apply(grant, &account, created)?;
    }
    Ok(())
}

fn access_exact(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> Result<bool, ServiceError> {
    let Some(receipt) = read_receipt(root, declaration)? else {
        return Ok(false);
    };
    if receipt.release != binding.relative(root).to_string_lossy() {
        return Ok(false);
    }
    let account = service_account(declaration);
    let plan = access_plan(root, declaration, binding);
    plan.iter().try_fold(true, |exact, grant| {
        Ok(exact && security::has_access(grant, &account)?)
    })
}

fn remove_access(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    let receipt = read_receipt(root, declaration)?
        .ok_or_else(|| ServiceError::invalid("access receipt is missing"))?;
    revoke_receipt(root, declaration, &receipt)
}

fn revoke_receipt(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    receipt: &AccessReceipt,
) -> Result<(), ServiceError> {
    let binding = Binding::admit(
        root,
        root.0.join("installs").join(&receipt.release),
        declaration,
    )?;
    let account = service_account(declaration);
    let plan = access_plan(root, declaration, &binding);
    for (grant, created) in plan.iter().zip(receipt.created) {
        security::revoke(grant, &account, created)?;
    }
    Ok(())
}

fn service_account(declaration: &NormalizedDecl) -> String {
    format!("NT SERVICE\\{}", declaration.id.as_str())
}

fn access_receipt(root: &ServiceRoot, declaration: &NormalizedDecl) -> std::path::PathBuf {
    root.directory(declaration).join("access.receipt")
}

fn read_receipt(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
) -> Result<Option<AccessReceipt>, ServiceError> {
    match std::fs::read_to_string(access_receipt(root, declaration)) {
        Ok(text) => parse_receipt(&text).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ServiceError::invalid(format!(
            "read access receipt: {error}"
        ))),
    }
}

fn parse_receipt(text: &str) -> Result<AccessReceipt, ServiceError> {
    let receipt: AccessReceipt = toml::from_str(text)
        .map_err(|error| ServiceError::invalid(format!("parse access receipt: {error}")))?;
    if receipt.schema == 1 {
        Ok(receipt)
    } else {
        Err(ServiceError::invalid("access receipt conflicts"))
    }
}

pub fn enable(_: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    configure(declaration, SERVICE_AUTO_START)
}
pub fn disable(_: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    configure(declaration, SERVICE_DEMAND_START)
}

pub fn rebind(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> Result<(), ServiceError> {
    apply_access(root, declaration, binding)?;
    let service = open(declaration, SERVICE_CHANGE_CONFIG)?;
    let command = wide(&render_definition(root, declaration, binding));
    win32(
        unsafe {
            ChangeServiceConfigW(
                service.0,
                SERVICE_NO_CHANGE,
                SERVICE_NO_CHANGE,
                SERVICE_NO_CHANGE,
                command.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        },
        "rebind service",
    )
}

pub fn start(_: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    let service = open(declaration, SERVICE_START | SERVICE_QUERY_STATUS)?;
    win32(
        unsafe { StartServiceW(service.0, 0, std::ptr::null()) },
        "start service",
    )?;
    wait_for(&service, SERVICE_RUNNING)
}

pub fn stop(_: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    let service = open(declaration, SERVICE_STOP | SERVICE_QUERY_STATUS)?;
    let mut status = SERVICE_STATUS::default();
    win32(
        unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) },
        "stop service",
    )?;
    wait_for(&service, SERVICE_STOPPED)
}

pub fn remove(root: &ServiceRoot, declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    let service = open(declaration, DELETE | SERVICE_QUERY_CONFIG)?;
    let (command, _, _) = query_config(&service)?;
    let binding = binding_from_command(root, declaration, &command)?;
    if command != render_definition(root, declaration, &binding) {
        return Err(ServiceError::invalid("service binding conflicts"));
    }
    win32(unsafe { DeleteService(service.0) }, "delete service")?;
    remove_access(root, declaration)
}

fn manager(access: u32) -> Result<ServiceHandle, ServiceError> {
    let raw = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), access) };
    if raw.is_null() {
        Err(last("open service manager"))
    } else {
        Ok(ServiceHandle(raw))
    }
}

fn open(declaration: &NormalizedDecl, access: u32) -> Result<ServiceHandle, ServiceError> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let name = wide(declaration.id.as_str());
    let raw = unsafe { OpenServiceW(manager.0, name.as_ptr(), access) };
    if raw.is_null() {
        Err(last("open service"))
    } else {
        Ok(ServiceHandle(raw))
    }
}

fn configure(declaration: &NormalizedDecl, start_type: u32) -> Result<(), ServiceError> {
    let service = open(declaration, SERVICE_CHANGE_CONFIG)?;
    win32(
        unsafe {
            ChangeServiceConfigW(
                service.0,
                SERVICE_NO_CHANGE,
                start_type,
                SERVICE_NO_CHANGE,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        },
        "configure service",
    )
}

fn query_runtime(service: &ServiceHandle) -> Result<Runtime, ServiceError> {
    let mut status = SERVICE_STATUS_PROCESS::default();
    let mut needed = 0;
    win32(
        unsafe {
            QueryServiceStatusEx(
                service.0,
                SC_STATUS_PROCESS_INFO,
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
                std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
                &mut needed,
            )
        },
        "query service status",
    )?;
    Ok(match status.dwCurrentState {
        SERVICE_STOPPED if status.dwWin32ExitCode == 0 => Runtime::Stopped,
        SERVICE_STOPPED => Runtime::Failed,
        SERVICE_START_PENDING => Runtime::Starting,
        SERVICE_RUNNING => Runtime::Running,
        SERVICE_STOP_PENDING => Runtime::Stopping,
        _ => Runtime::Failed,
    })
}

fn query_config(service: &ServiceHandle) -> Result<(String, u32, String), ServiceError> {
    let bytes = query_buffer("query service config", |buffer, size, needed| unsafe {
        QueryServiceConfigW(service.0, buffer.cast(), size, needed)
    })?;
    let config = unsafe { &*bytes.as_ptr().cast::<QUERY_SERVICE_CONFIGW>() };
    Ok((
        wide_string(config.lpBinaryPathName),
        config.dwStartType,
        wide_string(config.lpServiceStartName),
    ))
}

fn query_security(service: &ServiceHandle) -> Result<bool, ServiceError> {
    let sid = query_config2(service, SERVICE_CONFIG_SERVICE_SID_INFO)?;
    let sid = unsafe { &*sid.as_ptr().cast::<SERVICE_SID_INFO>() };
    let privileges = query_config2(service, SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO)?;
    let privileges = unsafe {
        &*privileges
            .as_ptr()
            .cast::<SERVICE_REQUIRED_PRIVILEGES_INFOW>()
    };
    let empty = privileges.pmszRequiredPrivileges.is_null()
        || unsafe { *privileges.pmszRequiredPrivileges == 0 };
    Ok(sid.dwServiceSidType == SID_TYPE && empty)
}

fn query_config2(service: &ServiceHandle, kind: u32) -> Result<Vec<usize>, ServiceError> {
    query_buffer(
        "query extended service configuration",
        |buffer, size, needed| unsafe {
            QueryServiceConfig2W(service.0, kind, buffer, size, needed)
        },
    )
}

fn query_buffer(
    action: &str,
    mut query: impl FnMut(*mut u8, u32, &mut u32) -> i32,
) -> Result<Vec<usize>, ServiceError> {
    let mut needed = 0;
    query(std::ptr::null_mut(), 0, &mut needed);
    let mut bytes = vec![0_usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    win32(
        query(bytes.as_mut_ptr().cast(), needed, &mut needed),
        action,
    )?;
    Ok(bytes)
}

fn wide_string(value: *const u16) -> String {
    if value.is_null() {
        return String::new();
    }
    let mut length = 0;
    unsafe {
        while *value.add(length) != 0 {
            length += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(value, length))
    }
}

fn wait_for(service: &ServiceHandle, expected: u32) -> Result<(), ServiceError> {
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(35) {
        let current = query_runtime(service)?;
        if (expected == SERVICE_RUNNING && current == Runtime::Running)
            || (expected == SERVICE_STOPPED && current == Runtime::Stopped)
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(ServiceError::invalid(
        "service transition exceeded 35 seconds",
    ))
}

pub(super) fn render_definition(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    binding: &Binding,
) -> String {
    let host = binding.host(declaration);
    format!(
        "\"{}\" \"{}\"",
        host.display(),
        root.declaration(declaration).display()
    )
}

fn binding_from_command(
    root: &ServiceRoot,
    declaration: &NormalizedDecl,
    command: &str,
) -> Result<Binding, ServiceError> {
    let suffix = format!("\" \"{}\"", root.declaration(declaration).display());
    let host = command
        .strip_prefix('"')
        .and_then(|command| command.strip_suffix(&suffix))
        .ok_or_else(|| ServiceError::invalid("service command shape conflicts"))?;
    Binding::from_host(root, Path::new(host), declaration)
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}
fn last(action: &str) -> ServiceError {
    ServiceError::invalid(format!("{action}: {}", std::io::Error::last_os_error()))
}

fn win32(result: i32, action: &str) -> Result<(), ServiceError> {
    if result == 0 {
        Err(last(action))
    } else {
        Ok(())
    }
}
