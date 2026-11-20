package com.sincy.agent.perception

import android.accessibilityservice.AccessibilityService
import android.content.pm.PackageManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import org.json.JSONArray
import org.json.JSONObject

/**
 * Observation + action layer (WP-11 / ADR-013: 控件树优先).
 *
 * Perception only reports the window content (node ids + texts) to the Rust
 * core for anchor matching. It NEVER injects gestures on its own — every action
 * goes through the signed adapter interpreter (step-level post-condition
 * checks, evidence capture). Challenge screens are detected by the interpreter
 * and parked as WAITING_USER; this service never tries to solve them.
 */
class SincyAccessibilityService : AccessibilityService() {

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        val root = rootInActiveWindow ?: return
        latestSnapshot = summarize(root)
        root.recycle()
    }

    override fun onInterrupt() = Unit

    companion object {
        @Volatile
        private var latestSnapshot: String = "{\"ids\":[],\"texts\":[]}"

        @Volatile
        var instance: SincyAccessibilityService? = null

        /** Current screen summary for the Rust core: {"ids": [...], "texts": [...]}. */
        fun currentSnapshotJson(): String = latestSnapshot

        /** Installed versionName of a package, for the adapter version_range guard. */
        fun installedVersion(packageName: String): String? {
            val svc = instance ?: return null
            return try {
                svc.packageManager.getPackageInfo(packageName, 0).versionName
            } catch (e: PackageManager.NameNotFoundException) {
                null
            }
        }

        private fun summarize(root: AccessibilityNodeInfo): String {
            val ids = JSONArray()
            val texts = JSONArray()
            fun walk(node: AccessibilityNodeInfo, depth: Int) {
                if (depth > 12) return
                node.viewIdResourceName?.let { ids.put(it) }
                node.text?.let { texts.put(it.toString()) }
                node.contentDescription?.let { texts.put(it.toString()) }
                for (i in 0 until node.childCount) {
                    node.getChild(i)?.let { walk(it, depth + 1) }
                }
            }
            walk(root, 0)
            return JSONObject().put("ids", ids).put("texts", texts).toString()
        }
    }

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
    }

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        instance = null
        return super.onUnbind(intent)
    }
}
