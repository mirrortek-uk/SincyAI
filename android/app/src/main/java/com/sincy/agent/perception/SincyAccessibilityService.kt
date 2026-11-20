package com.sincy.agent.perception

import android.accessibilityservice.AccessibilityService
import android.view.accessibility.AccessibilityEvent

/**
 * Observation layer (WP-11): exposes the window content (控件树) to the Rust
 * core for anchor matching. Perception only — it never injects gestures; all
 * actions go through the signed Adapter interpreter with step-level
 * post-condition checks and evidence capture.
 */
class SincyAccessibilityService : AccessibilityService() {

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        // TODO(WP-11): forward summarized node tree to Rust core via JNI
    }

    override fun onInterrupt() = Unit
}
