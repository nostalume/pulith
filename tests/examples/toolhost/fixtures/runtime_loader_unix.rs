use std::ffi::{CStr, c_char, c_void};

#[repr(C)]
struct DlInfo {
    filename: *const c_char,
    base: *mut c_void,
    symbol: *const c_char,
    address: *mut c_void,
}

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(name: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    fn dladdr(address: *const c_void, info: *mut DlInfo) -> i32;
}

fn main() {
    unsafe {
        let handle = dlopen(c"libexpected.so".as_ptr(), 2);
        assert!(!handle.is_null());
        let function = dlsym(handle, c"runtime_identity".as_ptr());
        assert!(!function.is_null());
        let identity = CStr::from_ptr(std::mem::transmute::<
            *mut c_void,
            unsafe extern "C" fn() -> *const c_char,
        >(function)());
        let mut info = DlInfo {
            filename: std::ptr::null(),
            base: std::ptr::null_mut(),
            symbol: std::ptr::null(),
            address: std::ptr::null_mut(),
        };
        assert_ne!(dladdr(function, &mut info), 0);
        let origin = std::path::PathBuf::from(CStr::from_ptr(info.filename).to_str().unwrap())
            .canonicalize()
            .unwrap();
        print!("identity={}\norigin={}\n", identity.to_str().unwrap(), origin.display());
    }
}
