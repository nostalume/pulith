#[unsafe(no_mangle)]
pub extern "C" fn runtime_identity() -> *const std::ffi::c_char {
    c"runtime/1".as_ptr()
}
