use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows_sys::Win32::System::Services::*;

static STOP: AtomicBool = AtomicBool::new(false);
static STATUS: AtomicIsize = AtomicIsize::new(0);

pub struct Control;

impl Control {
    pub fn arm() -> Result<Self, String> {
        STOP.store(false, Ordering::Release);
        Ok(Self)
    }

    pub fn ready(&self) -> Result<(), String> {
        set_status(SERVICE_RUNNING, SERVICE_ACCEPT_STOP)
    }

    pub fn stop_requested(&self) -> bool {
        STOP.load(Ordering::Acquire)
    }
}

pub fn dispatch() -> ExitCode {
    if std::env::args_os().nth(1).is_none() {
        return ExitCode::FAILURE;
    }
    let mut entries = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: std::ptr::null_mut(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW::default(),
    ];
    if unsafe { StartServiceCtrlDispatcherW(entries.as_mut_ptr()) } == 0 {
        eprintln!(
            "toolhost-service: dispatch: {}",
            std::io::Error::last_os_error()
        );
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

unsafe extern "system" fn service_main(count: u32, arguments: *mut windows_sys::core::PWSTR) {
    if count == 0 || arguments.is_null() {
        return;
    }
    let name = unsafe { *arguments };
    let handle = unsafe { RegisterServiceCtrlHandlerW(name, Some(control)) };
    if handle.is_null() {
        return;
    }
    STATUS.store(handle as isize, Ordering::Release);
    if set_status(SERVICE_START_PENDING, 0).is_err() {
        return;
    }
    let code = super::run_from_args();
    if code == ExitCode::SUCCESS {
        let _ = set_status(SERVICE_STOPPED, 0);
    } else {
        let _ = set_failed();
    }
}

unsafe extern "system" fn control(code: u32) {
    if code == SERVICE_CONTROL_STOP {
        STOP.store(true, Ordering::Release);
        let _ = set_status(SERVICE_STOP_PENDING, 0);
    }
}

fn set_status(state: u32, accepted: u32) -> Result<(), String> {
    let handle = STATUS.load(Ordering::Acquire) as SERVICE_STATUS_HANDLE;
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: accepted,
        dwWin32ExitCode: 0,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: 30_000,
    };
    if unsafe { SetServiceStatus(handle, &status) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

fn set_failed() -> Result<(), String> {
    let handle = STATUS.load(Ordering::Acquire) as SERVICE_STATUS_HANDLE;
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: SERVICE_STOPPED,
        dwControlsAccepted: 0,
        dwWin32ExitCode: 1066,
        dwServiceSpecificExitCode: 1,
        dwCheckPoint: 0,
        dwWaitHint: 0,
    };
    if unsafe { SetServiceStatus(handle, &status) } == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
