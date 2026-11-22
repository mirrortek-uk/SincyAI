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
        } ?: "tap,scroll,set_text,back,close_dialog"
        val screen = req.optString("screen")
        val historyArr = req.optJSONArray("history")
        val history = historyArr?.let { a ->
            (0 until a.length()).map { a.getString(it) }.joinToString("; ")
        } ?: ""
        val prompt = buildString {
            append("You control an Android phone through accessibility gestures to accomplish a user goal. ")
            append("Decide the SINGLE next action.\n")
            append("GOAL: ").append(goal).append('\n')
            append("STEP: ").append(step).append('/').append(maxSteps).append('\n')
            append("ALLOWED ACTIONS: ").append(allowed).append(", done, challenge\n")
            append("SCREEN (ids|texts|nodes with bounds): ").append(screen).append('\n')
            if (history.isNotEmpty()) append("HISTORY: ").append(history).append('\n')
            append("Reply with EXACTLY ONE JSON object and nothing else, one of:\n")
            append("""{"action":"tap","target":{"id":"<node id>"}} or {"action":"tap","target":{"text":"<visible text>"}}""")
            append('\n')
            append("""{"action":"scroll"} | {"action":"set_text","target":{"text":"<field text>"},"value":"<text>"}""")
            append('\n')
            append("""{"action":"back"} | {"action":"close_dialog"} | {"action":"done"} | {"action":"challenge","reason":"..."}""")
            append('\n')
            append("Pick tap targets from ids/texts actually visible on the SCREEN. Never invent actions outside ALLOWED ACTIONS.")
        }
        val base = cloudBase.ifEmpty { "http://127.0.0.1:18081" }
        val token = deviceToken
        val body = org.json.JSONObject()
            .put("prompt", prompt)
            .put("max_tokens", 256)
            .toString()
        val request = okhttp3.Request.Builder()
            .url("$base/api/v1/ai/generate")
            .header("Authorization", "Bearer $token")
            .post(okhttp3.RequestBody.create("application/json; charset=utf-8".toMediaType(), body))
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
