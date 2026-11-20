package com.sincy.agent.adapter

/**
 * Kotlin facade over the Rust adapter interpreter (rust-core via JNI).
 *
 * Contract with cloud (sincy-adapter): version guard first — if the target
 * app version is outside the adapter's version_range, refuse with zero side
 * effects and report ui_drift. Unknown screens follow the safe-recovery
 * whitelist (max 3 back navigations) then drift/waiting. Challenge screens
 * always park the task as WAITING_USER.
 */
object AdapterRuntime {
    init {
        System.loadLibrary("sincy_agent_core")
    }

    external fun runAdapter(adapterJson: String, appVersion: String): String
}
