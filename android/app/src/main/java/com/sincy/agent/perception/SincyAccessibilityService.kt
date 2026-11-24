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
        private var latestSnapshot: String = "{\"ids\":[],\"texts\":[],\"nodes\":[],\"width\":1080,\"height\":2400}"

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
         * {"kind":"tap|swipe|set_text|back|pick_media|close_dialog", ...}
         *
         * The Rust core sends CONCRETE humanized gesture plans (planned by
         * sincy-adapter::humanize): tap carries {x,y,hold_ms,pre_ms,post_ms},
         * swipe carries {segments:[{x1,y1,x2,y2,ms}],post_ms} — a bowed bezier
         * path with ease-in-out segment timing, executed as ONE continuous
         * multi-stroke dispatchGesture (willContinue). If coordinates are
         * absent we fall back to resolving "target" against the node tree.
         * Returns true when an input event was dispatched.
         */
        fun performAction(actionJson: String): Boolean {
            val svc = instance ?: return false
            val a = try { JSONObject(actionJson) } catch (e: Exception) { return false }
            val kind = a.optString("kind")
            val pre = a.optLong("pre_ms", 0)
            return when (kind) {
                "tap" -> {
                    if (a.has("x") && a.has("y")) {
                        sleepQuiet(pre)
                        val ok = tapAt(svc, a.getInt("x"), a.getInt("y"), a.optLong("hold_ms", 80))
                        sleepQuiet(a.optLong("post_ms", 0))
                        ok
                    } else {
                        tap(svc, a.optJSONObject("target"))
                    }
                }
                "swipe", "scroll" -> {
                    val segs = a.optJSONArray("segments")
                    val ok = if (segs != null && segs.length() > 0) swipePlan(svc, segs) else swipe(svc, up = true)
                    sleepQuiet(a.optLong("post_ms", 0))
                    ok
                }
                "set_text" -> {
                    sleepQuiet(pre)
                    setText(svc, a.optJSONObject("target"), a.optString("value"))
                }
                "back", "close_dialog", "pick_media" -> {
                    sleepQuiet(pre)
                    svc.performGlobalAction(AccessibilityService.GLOBAL_ACTION_BACK)
                }
                "wait" -> {
                    sleepQuiet(a.optLong("ms", 500))
                    true
                }
                // 纯感知 read: 值由 Rust 侧从 snapshot 直接读取, 这里不点击任何东西
                "read" -> true
                // 系统级 op (签名 system.* adapter): 下载/安装/轮询/验签/启动, 不走屏幕解释器
                "system_op" -> com.sincy.agent.setup.SetupOps.execute(svc, a.optJSONObject("op"))
                else -> false
            }
        }

        private fun sleepQuiet(ms: Long) {
            if (ms > 0) try { Thread.sleep(ms) } catch (_: InterruptedException) {}
        }

        private fun findNode(root: AccessibilityNodeInfo, target: JSONObject?): AccessibilityNodeInfo? {
            if (target == null) return null
            val id = target.optString("id").ifEmpty { null }
            val text = target.optString("text").ifEmpty { null }
            val textAny = target.optJSONArray("text_any")?.let { arr ->
                (0 until arr.length()).map { arr.getString(it) }
            }
            // Tap targets must be on-screen: hidden ViewPager pages (e.g. the other home
            // tab) also carry timeline_post nodes with bounds shifted off-screen; tapping
            // those coordinates is a no-op, so skip any node that does not intersect the display.
            val dm = instance?.resources?.displayMetrics
            val sw = dm?.widthPixels ?: 1080
            val sh = dm?.heightPixels ?: 2400
            val bounds = android.graphics.Rect()
            fun walk(node: AccessibilityNodeInfo, depth: Int): AccessibilityNodeInfo? {
                if (depth > 14) return null
                node.getBoundsInScreen(bounds)
                val onScreen = bounds.width() > 0 && bounds.height() > 0 &&
                    bounds.right > 0 && bounds.left < sw && bounds.bottom > 0 && bounds.top < sh
                val rid = node.viewIdResourceName
                val hay = listOfNotNull(rid, node.text?.toString(), node.contentDescription?.toString()).joinToString(" ")
                val idOk = id != null && rid != null &&
                    (rid == id || rid.endsWith("/$id") || rid.endsWith(":$id"))
                val textOk = text != null && (node.text?.toString() == text || node.contentDescription?.toString() == text)
                val anyOk = textAny != null && textAny.any { hay.contains(it) }
                if (onScreen && (idOk || textOk || anyOk || (id == null && text == null && textAny == null && node.isClickable))) {
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

        /** Real tap: 优先人性化手势点击节点中心 (真实触摸, canPerformGestures=true 时可用),
         *  手势失败再退回对可点击祖先做语义 ACTION_CLICK。 */
        private fun tap(svc: SincyAccessibilityService, target: JSONObject?): Boolean {
            val root = svc.rootInActiveWindow
            if (root == null) {
                android.util.Log.e("SincyAgent", "tap: rootInActiveWindow=null")
                return false
            }
            val node = findNode(root, target)
            if (node == null) {
                android.util.Log.e("SincyAgent", "tap: node not found for $target")
                root.recycle()
                return false
            }
            val r = android.graphics.Rect()
            node.getBoundsInScreen(r)
            val cx = r.centerX()
            val cy = r.centerY()
            android.util.Log.e("SincyAgent", "tap: target=$target at ($cx,$cy) bounds=$r")
            node.recycle()
            root.recycle()
            if (tapAt(svc, cx, cy, 80)) return true
            android.util.Log.e("SincyAgent", "tap: gesture failed at ($cx,$cy), fallback click")
            // 手势失败: 退回语义 click
            val root2 = svc.rootInActiveWindow ?: return false
            val node2 = findNode(root2, target) ?: run { root2.recycle(); return false }
            var n: AccessibilityNodeInfo? = node2
            var clickable: AccessibilityNodeInfo? = null
            while (n != null) {
                if (n.isClickable) { clickable = n; break }
                n = n.parent
            }
            val ok = clickable != null && clickable.performAction(AccessibilityNodeInfo.ACTION_CLICK)
            android.util.Log.e("SincyAgent", "tap: fallback click ok=$ok clickable=${clickable != null}")
            node2.recycle()
            root2.recycle()
            return ok
        }

        /** dispatchGesture click at an exact (already humanized) point, with a hold. */
        private fun tapAt(svc: SincyAccessibilityService, x: Int, y: Int, holdMs: Long): Boolean {
            repeat(2) { attempt ->
                if (attempt == 1) sleepQuiet(400) // 第一次失败可能是页面转场中, 稍作等待重试
                val path = Path().apply { moveTo(x.toFloat(), y.toFloat()); lineTo(x + 0.1f, y.toFloat()) }
                val stroke = GestureDescription.StrokeDescription(path, 0, holdMs.coerceAtLeast(30))
                val gesture = GestureDescription.Builder().addStroke(stroke).build()
                val dispatched = try {
                    svc.dispatchGesture(gesture, null, null)
                } catch (e: Exception) {
                    android.util.Log.e("SincyAgent", "dispatchGesture threw: ${e.message}")
                    false
                }
                if (dispatched) return true
            }
            android.util.Log.e("SincyAgent", "tapAt: gesture rejected at ($x,$y), fallback click")
            // fallback: semantic click on the deepest clickable node containing the point
            val root = svc.rootInActiveWindow ?: return false
            val hit = findClickableAt(root, x, y)
            root.recycle()
            return hit?.performAction(AccessibilityNodeInfo.ACTION_CLICK) ?: false
        }

        private fun findClickableAt(node: AccessibilityNodeInfo, x: Int, y: Int): AccessibilityNodeInfo? {
            val r = android.graphics.Rect()
            node.getBoundsInScreen(r)
            if (node.isClickable && r.contains(x, y)) return node
            for (i in 0 until node.childCount) {
                node.getChild(i)?.let { c -> findClickableAt(c, x, y)?.let { return it } }
            }
            return null
        }

        /** Set text into an editable node: focus + ACTION_SET_TEXT, clipboard paste as fallback. */
        private fun setText(svc: SincyAccessibilityService, target: JSONObject?, value: String): Boolean {
            val root = svc.rootInActiveWindow ?: return false
            var node = findNode(root, target)
            if (node != null && node.className?.toString()?.endsWith("EditText") != true) {
                // target matched a label/placeholder (e.g. 发布你的回复), not the input itself —
                // use the focused editable node instead
                node.recycle()
                node = root.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            }
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

        /** Real injected swipe (scroll) — legacy fixed path. */
        private fun swipe(svc: SincyAccessibilityService, up: Boolean): Boolean {
            val path = Path().apply {
                if (up) { moveTo(540f, 1500f); lineTo(540f, 700f) } else { moveTo(540f, 700f); lineTo(540f, 1500f) }
            }
            val stroke = GestureDescription.StrokeDescription(path, 0, 250)
            val gesture = GestureDescription.Builder().addStroke(stroke).build()
            return svc.dispatchGesture(gesture, null, null)
        }

        /**
         * Execute a humanized swipe plan as ONE continuous gesture: each segment
         * becomes a StrokeDescription with willContinue=true (except the last),
         * startTime offset by the cumulative segment durations — so the finger
         * follows the bowed, jittered path with ease-in-out speed instead of a
         * single straight constant-speed line.
         */
        private fun swipePlan(svc: SincyAccessibilityService, segs: JSONArray): Boolean {
            val builder = GestureDescription.Builder()
            var start = 0L
            for (i in 0 until segs.length()) {
                val s = segs.getJSONObject(i)
                val path = Path().apply {
                    moveTo(s.getInt("x1").toFloat(), s.getInt("y1").toFloat())
                    lineTo(s.getInt("x2").toFloat(), s.getInt("y2").toFloat())
                }
                val ms = s.optLong("ms", 50).coerceAtLeast(16)
                val willContinue = i < segs.length() - 1
                builder.addStroke(GestureDescription.StrokeDescription(path, start, ms, willContinue))
                start += ms
            }
            return svc.dispatchGesture(builder.build(), null, null)
        }

        private fun summarize(root: AccessibilityNodeInfo): String {
            val ids = JSONArray()
            val texts = JSONArray()
            val nodes = JSONArray()
            val dm = instance?.resources?.displayMetrics
            val width = dm?.widthPixels ?: 1080
            val height = dm?.heightPixels ?: 2400
            var elementIdx = 0
            val marks = mutableListOf<Triple<Int, Int, Int>>()
            fun walk(node: AccessibilityNodeInfo, depth: Int) {
                if (depth > 14) return
                val r = android.graphics.Rect()
                node.getBoundsInScreen(r)
                node.viewIdResourceName?.let { ids.put(it) }
                val t = node.text?.toString() ?: node.contentDescription?.toString()
                t?.let { texts.put(it) }
                val editable = node.className?.toString()?.endsWith("EditText") == true
                // Mobilerun-style element indexing: on-screen interactive nodes get a
                // stable 1-based idx so the LLM can tap by index instead of raw coords.
                val onScreen = r.width() > 0 && r.height() > 0 &&
                    r.right > 0 && r.left < width && r.bottom > 0 && r.top < height
                val idx = if (onScreen && (node.isClickable || editable) && elementIdx < 120) {
                    ++elementIdx
                } else 0
                if (idx > 0) marks.add(Triple(idx, (r.left + r.right) / 2, (r.top + r.bottom) / 2))
                nodes.put(
                    JSONObject()
                        .put("id", node.viewIdResourceName ?: "")
                        .put("text", node.text?.toString() ?: "")
                        .put("desc", node.contentDescription?.toString() ?: "")
                        .put("bounds", JSONArray(intArrayOf(r.left, r.top, r.right, r.bottom)))
                        .put("clickable", node.isClickable)
                        .put("focused", node.isFocused)
                        .put("editable", editable)
                        .put("idx", idx)
                )
                for (i in 0 until node.childCount) {
                    node.getChild(i)?.let { walk(it, depth + 1) }
                }
            }
            walk(root, 0)
            // 视觉通道: 若已授权录屏, 附带当前帧截图 (base64 jpeg, 带元素编号标注), 喂给 VLM
            val shot = try { ScreenCapture.captureBase64Jpeg(marks = marks.ifEmpty { null }) } catch (_: Exception) { null }
            return JSONObject()
                .put("ids", ids)
                .put("texts", texts)
                .put("nodes", nodes)
                .put("width", width)
                .put("height", height)
                .put("screenshot", shot ?: JSONObject.NULL)
                .toString()
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
