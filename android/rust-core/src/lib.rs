//! Rust core for the Android agent, exposed to Kotlin over JNI (ADR-002).
//! Reuses the exact cloud-side guarded state machines and adapter interpreter
//! so device behavior cannot diverge from server policy.

/// Entry point for the Kotlin `AdapterRuntime.runAdapter` JNI call.
/// `adapter_json` is the signed adapter definition; `app_version` is the
/// target app's version. Returns a JSON result envelope.
#[no_mangle]
pub extern "C" fn Java_com_sincy_agent_adapter_AdapterRuntime_runAdapter(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    _adapter_json: *mut std::ffi::c_void,
    _app_version: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    // TODO(WP-11): real JNI plumbing via jni crate; logic delegates to
    // sincy_adapter::interpreter with the device-side node tree supplied
    // from the accessibility layer. Never auto-solve challenges.
    std::ptr::null_mut()
}
