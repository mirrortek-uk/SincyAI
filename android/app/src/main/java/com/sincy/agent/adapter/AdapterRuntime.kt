package com.sincy.agent.adapter

/**
 * Kotlin facade over the Rust agent core (android/rust-core, JNI).
 *
 * The Rust core is the SAME sincy-agent code that host tests verify, so device
 * behavior cannot diverge from server policy:
 *  - HMAC signature over the canonical adapter JSON must verify under the
 *    pinned platform key, else the adapter is refused with ZERO side effects;
 *  - version_range must cover the installed app version, else ui_drift;
 *  - challenge screens park the task as WAITING_USER (never auto-solved);
 *  - after the side_effect step any failure is "uncertain" (no auto retry).
 *
 * `runAdapter` returns a JSON envelope:
 *   {"outcome": "success"|"no_side_effect_failure"|"uncertain"|"waiting_user"|"ui_drift"|"refused",
 *    "side_effect_done": bool, "adapter_id": str, "adapter_version": int, "events": [...]}
 */
object AdapterRuntime {

    /** true when the Rust core (libsincy_agent_core.so) is packaged and loaded. */
    val nativeLoaded: Boolean

    init {
        var loaded = false
        try {
            System.loadLibrary("sincy_agent_core")
            loaded = true
        } catch (e: UnsatisfiedLinkError) {
            // skeleton build without the cdylib: Kotlin shell (pairing/WSS/
            // accessibility) still works, adapter execution reports unavailable
            android.util.Log.w("AdapterRuntime", "native core not packaged: ${e.message}")
        }
        nativeLoaded = loaded
    }

    /**
     * @param adapterJson  signed adapter definition (canonical JSON from the cloud)
     * @param signature    platform HMAC signature over that JSON
     * @param appVersion   installed versionName of the target app
     */
    external fun runAdapter(adapterJson: String, signature: String, appVersion: String): String
}
