use std::ffi::{CStr, c_char, c_void};
use std::os::windows::ffi::OsStrExt;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(path: *const u16) -> *mut c_void;
    fn GetProcAddress(handle: *mut c_void, name: *const u8) -> *mut c_void;
    fn GetModuleFileNameW(handle: *mut c_void, path: *mut u16, length: u32) -> u32;
}

fn main() {
    let path = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("private-runtime/expected.dll");
    let wide = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    unsafe {
        let handle = LoadLibraryW(wide.as_ptr());
        assert!(!handle.is_null());
        let function = GetProcAddress(handle, c"runtime_identity".as_ptr() as _);
        assert!(!function.is_null());
        let identity = CStr::from_ptr(std::mem::transmute::<
            *mut c_void,
            unsafe extern "C" fn() -> *const c_char,
        >(function)());
        let mut buffer = [0_u16; 32768];
        let length = GetModuleFileNameW(handle, buffer.as_mut_ptr(), buffer.len() as u32);
        let origin = std::path::PathBuf::from(String::from_utf16(&buffer[..length as usize]).unwrap())
            .canonicalize()
            .unwrap();
        print!("identity={}\norigin={}\n", identity.to_str().unwrap(), origin.display());
    }
}
