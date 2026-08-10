use super::super::{NormalizedDecl, Runtime, ServiceError};
use std::os::windows::ffi::OsStrExt;
use windows_sys::Win32::Storage::FileSystem::DELETE;
use windows_sys::Win32::System::Services::*;

pub(crate) const OBSERVE_ACCESS: u32 = SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS;
pub(crate) const BINDING_ACCESS: u32 = SERVICE_QUERY_CONFIG;
pub(crate) const REPAIR_ACCESS: u32 =
    SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS;
pub(crate) const CONFIGURE_ACCESS: u32 = SERVICE_CHANGE_CONFIG;
pub(crate) const REBIND_ACCESS: u32 = SERVICE_CHANGE_CONFIG;
pub(crate) const START_ACCESS: u32 = SERVICE_START | SERVICE_QUERY_STATUS;
pub(crate) const STOP_ACCESS: u32 = SERVICE_STOP | SERVICE_QUERY_STATUS;
pub(crate) const REMOVE_ACCESS: u32 = DELETE | SERVICE_QUERY_CONFIG;
pub(crate) const CREATED_ACCESS: u32 = REPAIR_ACCESS;
pub(crate) const SID_TYPE: u32 = 3;
pub(crate) const REQUIRED_PRIVILEGES: &[&str] = &[];

struct ManagerHandle(SC_HANDLE);

pub(super) struct ServiceHandle(SC_HANDLE);

pub(super) enum OpenedService {
    Missing,
    Removing,
    Present(ServiceHandle),
}

pub(super) struct ServiceConfig {
    pub command: String,
    pub start_type: u32,
    pub account: String,
}

impl Drop for ManagerHandle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0) };
    }
}

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0) };
    }
}

pub(super) fn observe(declaration: &NormalizedDecl) -> Result<OpenedService, ServiceError> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let name = wide(declaration.id.as_str());
    let raw = unsafe { OpenServiceW(manager.0, name.as_ptr(), OBSERVE_ACCESS) };
    if !raw.is_null() {
        return Ok(OpenedService::Present(ServiceHandle(raw)));
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(1060) => Ok(OpenedService::Missing),
        Some(1072) => Ok(OpenedService::Removing),
        _ => Err(last("open service")),
    }
}

macro_rules! openers {
    ($($name:ident = $access:ident),+ $(,)?) => {$(
        pub(super) fn $name(declaration: &NormalizedDecl) -> Result<ServiceHandle, ServiceError> {
            open(declaration, $access)
        }
    )+};
}
openers! {
    binding = BINDING_ACCESS, repair = REPAIR_ACCESS, configure = CONFIGURE_ACCESS,
    rebind = REBIND_ACCESS, start = START_ACCESS, stop = STOP_ACCESS,
}

pub(super) enum Removal {
    Missing,
    Removing,
    Present(ServiceHandle),
}

pub(super) fn observe_for_removal(declaration: &NormalizedDecl) -> Result<Removal, ServiceError> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let name = wide(declaration.id.as_str());
    let raw = unsafe { OpenServiceW(manager.0, name.as_ptr(), REMOVE_ACCESS) };
    if !raw.is_null() {
        return Ok(Removal::Present(ServiceHandle(raw)));
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(1060) => Ok(Removal::Missing),
        Some(1072) => Ok(Removal::Removing),
        _ => Err(last("open service for removal")),
    }
}

pub(super) fn await_missing(declaration: &NormalizedDecl) -> Result<(), ServiceError> {
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(35) {
        match observe(declaration)? {
            OpenedService::Missing => return Ok(()),
            OpenedService::Removing => std::thread::sleep(std::time::Duration::from_millis(100)),
            OpenedService::Present(_) => {
                return Err(ServiceError::invalid("service deletion conflicts"));
            }
        }
    }
    Err(ServiceError::operation(
        "service deletion",
        "exceeded 35 seconds",
    ))
}

pub(super) fn create(
    declaration: &NormalizedDecl,
    command: &str,
    account: &str,
) -> Result<ServiceHandle, ServiceError> {
    let manager = manager(SC_MANAGER_CREATE_SERVICE)?;
    let name = wide(declaration.id.as_str());
    let command = wide(command);
    let account = wide(account);
    let raw = unsafe {
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
    (!raw.is_null())
        .then_some(ServiceHandle(raw))
        .ok_or_else(|| last("create service"))
}

impl ServiceHandle {
    pub(super) fn config(&self) -> Result<ServiceConfig, ServiceError> {
        let bytes = self.query_buffer("query service config", |buffer, size, needed| unsafe {
            QueryServiceConfigW(self.0, buffer.cast(), size, needed)
        })?;
        let config = unsafe { &*bytes.as_ptr().cast::<QUERY_SERVICE_CONFIGW>() };
        Ok(ServiceConfig {
            command: wide_string(config.lpBinaryPathName),
            start_type: config.dwStartType,
            account: wide_string(config.lpServiceStartName),
        })
    }

    pub(super) fn runtime(&self) -> Result<Runtime, ServiceError> {
        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut needed = 0;
        win32(
            unsafe {
                QueryServiceStatusEx(
                    self.0,
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

    pub(super) fn security_is_exact(&self) -> Result<bool, ServiceError> {
        let sid = self.query_config2(SERVICE_CONFIG_SERVICE_SID_INFO)?;
        let sid = unsafe { &*sid.as_ptr().cast::<SERVICE_SID_INFO>() };
        let privileges = self.query_config2(SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO)?;
        let privileges = unsafe {
            &*privileges
                .as_ptr()
                .cast::<SERVICE_REQUIRED_PRIVILEGES_INFOW>()
        };
        let empty = privileges.pmszRequiredPrivileges.is_null()
            || unsafe { *privileges.pmszRequiredPrivileges == 0 };
        Ok(sid.dwServiceSidType == SID_TYPE && empty)
    }

    pub(super) fn configure_security(&self) -> Result<(), ServiceError> {
        let sid = SERVICE_SID_INFO {
            dwServiceSidType: SID_TYPE,
        };
        self.set_config2(SERVICE_CONFIG_SERVICE_SID_INFO, &sid)?;
        let mut empty = [0_u16; 2];
        let privileges = SERVICE_REQUIRED_PRIVILEGES_INFOW {
            pmszRequiredPrivileges: empty.as_mut_ptr(),
        };
        self.set_config2(SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO, &privileges)
    }

    pub(super) fn enable(&self) -> Result<(), ServiceError> {
        self.change(SERVICE_AUTO_START, std::ptr::null(), "configure service")
    }

    pub(super) fn disable(&self) -> Result<(), ServiceError> {
        self.change(SERVICE_DEMAND_START, std::ptr::null(), "configure service")
    }

    pub(super) fn set_binding(&self, command: &str) -> Result<(), ServiceError> {
        let command = wide(command);
        self.change(SERVICE_NO_CHANGE, command.as_ptr(), "rebind service")
    }

    fn change(
        &self,
        start_type: u32,
        command: *const u16,
        action: &'static str,
    ) -> Result<(), ServiceError> {
        win32(
            unsafe {
                ChangeServiceConfigW(
                    self.0,
                    SERVICE_NO_CHANGE,
                    start_type,
                    SERVICE_NO_CHANGE,
                    command,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                )
            },
            action,
        )
    }

    pub(super) fn start(&self) -> Result<(), ServiceError> {
        win32(
            unsafe { StartServiceW(self.0, 0, std::ptr::null()) },
            "start service",
        )?;
        self.wait_for(SERVICE_RUNNING)
    }

    pub(super) fn stop(&self) -> Result<(), ServiceError> {
        let mut status = SERVICE_STATUS::default();
        win32(
            unsafe { ControlService(self.0, SERVICE_CONTROL_STOP, &mut status) },
            "stop service",
        )?;
        self.wait_for(SERVICE_STOPPED)
    }

    pub(super) fn delete(self) -> Result<(), ServiceError> {
        let result = win32(unsafe { DeleteService(self.0) }, "delete service");
        drop(self);
        result
    }

    fn set_config2<T>(&self, kind: u32, value: &T) -> Result<(), ServiceError> {
        win32(
            unsafe { ChangeServiceConfig2W(self.0, kind, (value as *const T).cast()) },
            "configure service security",
        )
    }

    fn query_config2(&self, kind: u32) -> Result<Vec<usize>, ServiceError> {
        self.query_buffer(
            "query extended service configuration",
            |buffer, size, needed| unsafe {
                QueryServiceConfig2W(self.0, kind, buffer, size, needed)
            },
        )
    }

    fn query_buffer(
        &self,
        action: &'static str,
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

    fn wait_for(&self, expected: u32) -> Result<(), ServiceError> {
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(35) {
            let current = self.runtime()?;
            if (expected == SERVICE_RUNNING && current == Runtime::Running)
                || (expected == SERVICE_STOPPED && current == Runtime::Stopped)
            {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(ServiceError::operation(
            "service transition",
            "exceeded 35 seconds",
        ))
    }
}

fn manager(access: u32) -> Result<ManagerHandle, ServiceError> {
    let raw = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), access) };
    (!raw.is_null())
        .then_some(ManagerHandle(raw))
        .ok_or_else(|| last("open service manager"))
}

fn open(declaration: &NormalizedDecl, access: u32) -> Result<ServiceHandle, ServiceError> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let name = wide(declaration.id.as_str());
    let raw = unsafe { OpenServiceW(manager.0, name.as_ptr(), access) };
    (!raw.is_null())
        .then_some(ServiceHandle(raw))
        .ok_or_else(|| last("open service"))
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
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

fn last(action: &'static str) -> ServiceError {
    ServiceError::os(action, std::io::Error::last_os_error())
}

fn win32(result: i32, action: &'static str) -> Result<(), ServiceError> {
    if result == 0 {
        Err(last(action))
    } else {
        Ok(())
    }
}
