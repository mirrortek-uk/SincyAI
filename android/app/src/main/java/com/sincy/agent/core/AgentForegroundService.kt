package com.sincy.agent.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.IBinder
import androidx.core.app.NotificationCompat
import com.sincy.agent.Prefs
import com.sincy.agent.R
import com.sincy.agent.adapter.AdapterRuntime
import com.sincy.agent.net.AgentGateway
import com.sincy.agent.perception.SincyAccessibilityService
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject
import java.util.UUID

/**
 * Long-running agent process. Owns the WSS control channel (net/AgentGateway)
 * and drives the adapter interpreter via the Rust core (JNI, AdapterRuntime).
 *
 * Hard rules (Implementation Guide / HID design doc):
 *  - never auto-solve challenge screens — report challenge.detected, task parks
 *    as WAITING_USER;
 *  - never run unsigned adapters or adapters whose version_range does not cover
 *    the installed app version (the Rust core refuses with zero side effects);
 *  - after the side_effect step, report "uncertain" — never auto-retry/degrade.
 */
class AgentForegroundService : Service() {

    private val scope = CoroutineScope(Dispatchers.IO + Job())
    private var ws: WebSocket? = null
    private var heartbeatJob: Job? = null
    private val http = OkHttpClient()

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Sincy Agent", NotificationManager.IMPORTANCE_LOW))
        notify("启动中…")
        connect()
    }

    private fun prefs() = Prefs.get(this)

    private fun notify(text: String) {
        val nm = getSystemService(NotificationManager::class.java)
        val n = NotificationCompat.Builder(this, CHANNEL)
            .setContentTitle("Sincy Agent v${com.sincy.agent.BuildConfig.VERSION_NAME}")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .setOngoing(true)
            .build()
        nm.notify(NOTIF_ID, n)
        try { startForeground(NOTIF_ID, n) } catch (_: Exception) {}
    }

    private fun connect() {
        val token = prefs().getString("device_token", null)
        if (token.isNullOrEmpty()) { notify("未配对 — 请先在 app 里用配对码配对"); return stopSelf() }
        val baseUrl = prefs().getString("cloud_url", "http://127.0.0.1:18081")!!
        // agentic loop 的 LLM 决策回调用 (AdapterRuntime.llmDecide)
        AdapterRuntime.cloudBase = baseUrl
        AdapterRuntime.deviceToken = token
        notify("连接中 $baseUrl …")
        val gateway = AgentGateway(baseUrl)
        ws = gateway.connect(token, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: okhttp3.Response) {
                notify("● online — WSS 已连接")
                heartbeatJob = scope.launch {
                    while (isActive) {
                        send(webSocket, "device.heartbeat", JSONObject().put("status", "online"))
                        delay(10_000)
                    }
                }
            }
            override fun onMessage(webSocket: WebSocket, text: String) {
                val env = JSONObject(text)
                when (env.optString("message_type")) {
                    "device.task.assign" -> scope.launch { handleAssign(webSocket, baseUrl, token, env.getJSONObject("payload").getString("task_id")) }
                    "device.session.revoke" -> stopSelf()
                }
            }
            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                notify("连接断开 ($code) — 5s 后重连")
                scheduleReconnect()
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: okhttp3.Response?) {
                notify("连接失败 (HTTP ${response?.code ?: "?"}) ${t.message ?: ""} — 5s 后重连")
                scheduleReconnect()
            }
        })
    }

    private fun scheduleReconnect() {
        scope.launch { delay(5_000); connect() }
    }

    private suspend fun handleAssign(webSocket: WebSocket, baseUrl: String, token: String, taskId: String) {
        // fetch the signed task brief (package text + adapter + signature) over HTTP.
        // 注意必须同时带 ?token= (handler 设备鉴权) 和 Authorization: Bearer 头 (auth middleware),
        // 参考 crates/sincy-agent/src/client.rs
        val resp = http.newCall(Request.Builder()
            .url("$baseUrl/api/v1/agent/tasks/$taskId?token=$token")
            .header("Authorization", "Bearer $token")
            .get().build()).execute()
        if (!resp.isSuccessful) {
            // 拉不到简报绝不能静默 —— 上报失败, 否则任务永远卡 EXECUTING
            val body = resp.body?.string()
            send(webSocket, "device.task.result", JSONObject()
                .put("task_id", taskId)
                .put("outcome", "no_side_effect_failure")
                .put("reason", "brief fetch HTTP ${resp.code}: ${body?.take(120)}"))
            return
        }
        val brief = JSONObject(resp.body!!.string())
        resp.close()
        send(webSocket, "device.task.ack", JSONObject().put("task_id", taskId).put("state", "EXECUTING"))

        // run the signed adapter through the Rust core; the accessibility service
        // supplies the live node tree, the interpreter decides every action
        val appVersion = SincyAccessibilityService.installedVersion(brief.getString("app_package")) ?: "0"
        // agentic 任务的 goal 在 task_context 里 (数据, 非签名参数)
        val taskContext = brief.optJSONObject("task_context")?.toString() ?: "{}"
        val resultJson = AdapterRuntime.runAdapter(
            brief.getString("adapter_yaml"),
            brief.getString("signature"),
            appVersion,
            taskContext,
        )
        val result = JSONObject(resultJson)

        // stream step evidence
        val events = result.optJSONArray("events")
        if (events != null) {
            for (i in 0 until events.length()) {
                send(webSocket, "device.step.evidence", JSONObject()
                    .put("task_id", taskId).put("event", events.getString(i)))
            }
        }

        when (result.getString("outcome")) {
            "waiting_user" -> send(webSocket, "device.challenge.detected", JSONObject()
                .put("task_id", taskId).put("reason", "challenge screen — parked for human"))
            "ui_drift" -> send(webSocket, "device.ui_drift.detected", JSONObject()
                .put("task_id", taskId)
                .put("platform", brief.getString("platform"))
                .put("action", brief.getString("action"))
                .put("reason", "adapter/app version mismatch"))
        }
        send(webSocket, "device.task.result", JSONObject()
            .put("task_id", taskId)
            .put("outcome", result.getString("outcome"))
            .put("side_effect_done", result.optBoolean("side_effect_done")))
    }

    private fun send(webSocket: WebSocket, type: String, payload: JSONObject) {
        val env = JSONObject()
            .put("message_id", UUID.randomUUID().toString())
            .put("correlation_id", UUID.randomUUID().toString())
            .put("sent_at", java.time.Instant.now().toString())
            .put("schema_version", 1)
            .put("tenant_id", prefs().getString("agency_id", "") ?: "")
            .put("device_id", prefs().getString("device_id", "") ?: "")
            .put("message_type", type)
            .put("payload", payload)
        webSocket.send(env.toString())
    }

    override fun onDestroy() {
        heartbeatJob?.cancel()
        ws?.close(1000, "bye")
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        const val CHANNEL = "sincy.agent"
        const val NOTIF_ID = 1
    }
}
