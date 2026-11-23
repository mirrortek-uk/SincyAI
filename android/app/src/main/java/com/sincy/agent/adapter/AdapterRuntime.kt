package com.sincy.agent.adapter

import okhttp3.MediaType.Companion.toMediaType

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
     * @param adapterJson     signed adapter definition (canonical JSON from the cloud)
     * @param signature       platform HMAC signature over that JSON
     * @param appVersion      installed versionName of the target app
     * @param taskContextJson task context JSON (e.g. {"goal": "..."} for agentic tasks); "" = none
     */
    external fun runAdapter(adapterJson: String, signature: String, appVersion: String, taskContextJson: String): String

    /** Cloud base URL + device token, set by AgentForegroundService on connect; used by llmDecide. */
    @Volatile var cloudBase: String = ""
    @Volatile var deviceToken: String = ""

    private val llmHttp = okhttp3.OkHttpClient()

    /**
     * Called by the Rust core (agentic loop) to ask the cloud LLM for the next
     * action. Builds a compact screen+goal prompt and POSTs /api/v1/ai/generate
     * with the device token; returns the raw reply text (Rust extracts the JSON).
     */
    @Suppress("unused")
    fun llmDecide(reqJson: String): String {
        val req = org.json.JSONObject(reqJson)
        val goal = req.optString("goal")
        val step = req.optInt("step", 1)
        val maxSteps = req.optInt("max_steps", 30)
        val allowed = req.optJSONArray("allowed_actions")?.let { a ->
            (0 until a.length()).map { a.getString(it) }.joinToString(",")
        } ?: "tap,tap_xy,scroll,swipe_xy,set_text,back,close_dialog,wait"
        val screen = req.optString("screen")
        val elements = req.optString("elements")
        val hasImage = req.optString("image").isNotEmpty()
        val model = req.optString("model")
        val historyArr = req.optJSONArray("history")
        val history = historyArr?.let { a ->
            (0 until a.length()).map { a.getString(it) }.joinToString("; ")
        } ?: ""
        val prompt = buildString {
            append("You are a vision-capable Android phone agent (like DroidRun). You control the phone through touch gestures to accomplish the user's goal. ")
            append("Decide the SINGLE next action.\n")
            if (hasImage) append("A SCREENSHOT of the current phone screen is attached with RED NUMBERED CIRCLES drawn on every interactive element — the numbers match the ELEMENTS list idx. Use tap_element with the number you see on the target. Coordinates are in full-screen pixel space (screen width/height are in SCREEN header).\n")
            append("GOAL: ").append(goal).append('\n')
            append("STEP: ").append(step).append('/').append(maxSteps).append('\n')
            append("ALLOWED ACTIONS: ").append(allowed).append(", done, challenge\n")
            append("SCREEN (a11y tree ids|texts|nodes with bounds): ").append(screen).append('\n')
            if (elements.isNotEmpty()) append("ELEMENTS (numbered interactive elements, tap by index):\n").append(elements).append('\n')
            if (history.isNotEmpty()) append("HISTORY: ").append(history).append('\n')
            append("Reply with EXACTLY ONE JSON object and nothing else, one of:\n")
            append("""{"action":"tap_element","element":12}  — tap interactive element #12 from the ELEMENTS list (preferred: unambiguous, index-based)""")
            append('\n')
            append("""{"action":"tap_xy","x":540,"y":1200}  — tap raw screenshot coordinates (use when the target is visible in the image but not in ELEMENTS)""")
            append('\n')
            append("""{"action":"tap","target":{"id":"<resource-id>"}} or {"action":"tap","target":{"text":"<visible text>"}}  — tap by a11y node""")
            append('\n')
            append("""{"action":"swipe_xy","x1":540,"y1":1600,"x2":540,"y2":600,"ms":350} | {"action":"scroll"}  — scroll/swipe""")
            append('\n')
            append("""{"action":"set_text","target":{"text":"<field placeholder text>"},"value":"<text>"} — device writes into the focused input even if target only matches the placeholder | {"action":"wait","ms":1000}""")
            append('\n')
            append("""{"action":"back"} | {"action":"close_dialog"} | {"action":"launch_app"} | {"action":"done"} | {"action":"done","answer":"..."} (report findings: e.g. the full text of a post you were asked to read) | {"action":"challenge","reason":"..."}""")
            append('\n')
            append("RULES: (1) If a HISTORY line says screen UNCHANGED, do NOT repeat that action — scroll or pick a different target. ")
            append("(2) When the GOAL is visibly achieved, reply {\"action\":\"done\"}. ")
            append("(3) Prefer tap_element with an idx from ELEMENTS (reliable); use tap_xy only for targets visible in the SCREENSHOT but missing from ELEMENTS. ")
            append("(4) If the screen shows the phone launcher / a different app than the goal needs (app was killed in background), reply {\"action\":\"launch_app\"} once to bring the target app back. ")
            append("(5) If stuck after a few steps, reply {\"action\":\"challenge\",\"reason\":\"stuck: ...\"} instead of looping. ")
            append("(6) Never invent actions outside ALLOWED ACTIONS — no installs, no posts, no payments; those are refused by policy.")
        }
        val base = cloudBase.ifEmpty { "http://127.0.0.1:18081" }
        val token = deviceToken
        val body = org.json.JSONObject()
            .put("prompt", prompt)
            .put("max_tokens", 384)
        if (model.isNotEmpty()) body.put("model", model)
        if (hasImage) body.put("images", org.json.JSONArray().put(req.getString("image")))
        val request = okhttp3.Request.Builder()
            .url("$base/api/v1/ai/generate")
            .header("Authorization", "Bearer $token")
            .post(okhttp3.RequestBody.create("application/json; charset=utf-8".toMediaType(), body.toString()))
            .build()
        llmHttp.newCall(request).execute().use { resp ->
            val respBody = resp.body?.string() ?: ""
            // 传输/鉴权/预算错误不是人机挑战: 用白名单外的 action 让 Rust 侧判 FAILED,
            // 避免基础设施故障被误判为 WAITING_USER 挂起
            if (!resp.isSuccessful) return "{\"action\":\"llm_error\",\"reason\":\"llm http ${resp.code}: ${respBody.take(120)}\"}"
            return try {
                org.json.JSONObject(respBody).optString("text", "")
            } catch (e: Exception) {
                "{\"action\":\"llm_error\",\"reason\":\"bad llm response: ${e.message}\"}"
            }
        }
    }

    /** Called by the Rust core before each interpreter step: current screen summary. */
    fun snapshotJson(): String =
        com.sincy.agent.perception.SincyAccessibilityService.currentSnapshotJson()

    /**
     * Called by the Rust core for each step action; the accessibility service
     * resolves the anchor to a node and injects a real touch (dispatchGesture).
     */
    fun performAction(actionJson: String): Boolean =
        com.sincy.agent.perception.SincyAccessibilityService.performAction(actionJson)

    /** system_op 执行结果字符串, 由 Rust JNI 通过 lastOpResult() 读回。 */
    @Volatile
    var lastOpResult: String = "ok"

    /** Called by the Rust core after a system op to read its result string. */
    fun lastOpResult(): String = lastOpResult
}
