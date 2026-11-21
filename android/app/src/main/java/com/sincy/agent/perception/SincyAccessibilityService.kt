package com.sincy.agent.perception

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Path
import android.os.Bundle
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import org.json.JSONArray
import org.json.JSONObject

/**
 * Observation + action layer (WP-11 / ADR-013: 控件树优先).
 *
 * Perception reports the window content (node ids + texts + bounds) to the Rust
 * core for anchor matching. Actions are ONLY performed here, and only as dictated
 * by the signed adapter interpreter running in the Rust core (step-level
 * post-condition checks, evidence capture) — this service never decides on its
 * own and never injects input for challenge screens (the interpreter parks those
 * as WAITING_USER).
 *
 * Touch injection = AccessibilityService.dispatchGesture (real injected touch
 * events at node coordinates), with performAction(ACTION_CLICK) as fallback.
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
        private var latestSnapshot: String = "{\"ids\":[],\"texts\":[],\"nodes\":[]}"

        @Volatile
        var instance: SincyAccessibilityService? = null

        /** Current screen summary for the Rust core: ids/texts for anchor match + nodes with bounds for touch. */
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

        /**
         * Perform one interpreter action. actionJson =
         * {"kind":"tap|set_text|back|scroll|pick_media|close_dialog",
         *  "target":{"id","text","text_any"}, "value":...}
         * Returns true when an input event was dispatched.
         */
        fun performAction(actionJson: String): Boolean {
            val svc = instance ?: return false
            val a = try { JSONObject(actionJson) } catch (e: Exception) { return false }
            val kind = a.optString("kind")
            val target = a.optJSONObject("target")
            return when (kind) {
                "tap" -> tap(svc, target)
                "set_text" -> setText(svc, target, a.optString("value"))
                "back", "close_dialog", "pick_media" -> svc.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
                "scroll" -> swipe(svc, up = true)
                else -> false
            }
        }

        private fun findNode(root: AccessibilityNodeInfo, target: JSONObject?): AccessibilityNodeInfo? {
            if (target == null) return null
            val id = target.optString("id").ifEmpty { null }
            val text = target.optString("text").ifEmpty { null }
            val textAny = target.optJSONArray("text_any")?.let { arr ->
                (0 until arr.length()).map { arr.getString(it) }
            }
            fun walk(node: AccessibilityNodeInfo, depth: Int): AccessibilityNodeInfo? {
                if (depth > 14) return null
                val rid = node.viewIdResourceName
                val hay = listOfNotNull(rid, node.text?.toString(), node.contentDescription?.toString()).joinToString(" ")
                val idOk = id != null && rid != null &&
                    (rid == id || rid.endsWith("/$id") || rid.endsWith(":$id"))
                val textOk = text != null && (node.text?.toString() == text || node.contentDescription?.toString() == text)
                val anyOk = textAny != null && textAny.any { hay.contains(it) }
                if (idOk || textOk || anyOk || (id == null && text == null && textAny == null && node.isClickable)) {
                    return node
                }
                for (i in 0 until node.childCount) {
                    node.getChild(i)?.let { child ->
                        walk(child, depth + 1)?.let { return it }
                    }
                }
                return null
            }
            return walk(root, 0)
        }

        /** Real injected touch: dispatchGesture click at the node center. */
        private fun tap(svc: SincyAccessibilityService, target: JSONObject?): Boolean {
            val root = svc.rootInActiveWindow ?: return false
            val node = findNode(root, target)
            if (node != null) {
                val r = android.graphics.Rect()
                node.getBoundsInScreen(r)
                val cx = r.centerX().toFloat()
                val cy = r.centerY().toFloat()
                root.recycle()
                val path = Path().apply { moveTo(cx, cy); lineTo(cx + 0.1f, cy) }
                val stroke = GestureDescription.StrokeDescription(path, 0, 80)
                val gesture = GestureDescription.Builder().addStroke(stroke).build()
                val dispatched = svc.dispatchGesture(gesture, null, null)
                if (dispatched) return true
                // fallback: semantic click on the node itself
                return node.performAction(AccessibilityNodeInfo.ACTION_CLICK)
            }
            root.recycle()
            return false
        }

        /** Set text into an editable node: focus + ACTION_SET_TEXT, clipboard paste as fallback. */
        private fun setText(svc: SincyAccessibilityService, target: JSONObject?, value: String): Boolean {
            val root = svc.rootInActiveWindow ?: return false
            var node = findNode(root, target)
            if (node == null) {
                // fall back to the currently focused editable node
                node = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            }
            if (node == null) { root.recycle(); return false }
            node.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
            val args = Bundle().apply {
                putCharSequence(
                    AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                    value
                )
            }
            val ok = node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
            if (!ok) {
                // clipboard paste fallback
                val cm = svc.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                cm.setPrimaryClip(ClipData.newPlainText("sincy", value))
                node.performAction(AccessibilityNodeInfo.ACTION_PASTE)
            }
            root.recycle()
            return true
        }

        /** Real injected swipe (scroll). */
        private fun swipe(svc: SincyAccessibilityService, up: Boolean): Boolean {
            val path = Path().apply {
                if (up) { moveTo(540f, 1500f); lineTo(540f, 700f) } else { moveTo(540f, 700f); lineTo(540f, 1500f) }
            }
            val stroke = GestureDescription.StrokeDescription(path, 0, 250)
            val gesture = GestureDescription.Builder().addStroke(stroke).build()
            return svc.dispatchGesture(gesture, null, null)
        }

        private fun summarize(root: AccessibilityNodeInfo): String {
            val ids = JSONArray()
            val texts = JSONArray()
            val nodes = JSONArray()
            fun walk(node: AccessibilityNodeInfo, depth: Int) {
                if (depth > 14) return
                val r = android.graphics.Rect()
                node.getBoundsInScreen(r)
                node.viewIdResourceName?.let { ids.put(it) }
                val t = node.text?.toString() ?: node.contentDescription?.toString()
                t?.let { texts.put(it) }
                nodes.put(
                    JSONObject()
                        .put("id", node.viewIdResourceName ?: "")
                        .put("text", node.text?.toString() ?: "")
                        .put("desc", node.contentDescription?.toString() ?: "")
                        .put("bounds", JSONArray(intArrayOf(r.left, r.top, r.right, r.bottom)))
                        .put("clickable", node.isClickable)
                        .put("focused", node.isFocused)
                        .put("editable", node.className?.toString()?.endsWith("EditText") == true)
                )
                for (i in 0 until node.childCount) {
                    node.getChild(i)?.let { walk(it, depth + 1) }
                }
            }
            walk(root, 0)
            return JSONObject().put("ids", ids).put("texts", texts).put("nodes", nodes).toString()
        }
    }

    override fun onServiceConnected() {
        super.onServiceConnected()
        instance = this
    }

    override fun onUnbind(intent: Intent?): Boolean {
        instance = null
        return super.onUnbind(intent)
    }
}
