package com.sincy.agent.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.IBinder
import androidx.core.app.NotificationCompat
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
        startForeground(NOTIF_ID, NotificationCompat.Builder(this, CHANNEL)
            .setContentTitle("Sincy Agent")
            .setContentText("connected to cloud")
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .build())
        connect()
    }

    private fun prefs() = getSharedPreferences("sincy_agent_prefs", MODE_PRIVATE)

    private fun connect() {
        val token = prefs().getString("device_token", null) ?: return stopSelf()
        val baseUrl = prefs().getString("cloud_url", "http://10.0.2.2:18080")!!
        val gateway = AgentGateway(baseUrl)
        ws = gateway.connect(token, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: okhttp3.Response) {
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
            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) { scheduleReconnect() }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: okhttp3.Response?) { scheduleReconnect() }
        })
    }

    private fun scheduleReconnect() {
        scope.launch { delay(5_000); connect() }
    }

    private suspend fun handleAssign(webSocket: WebSocket, baseUrl: String, token: String, taskId: String) {
        // fetch the signed task brief (package text + adapter + signature) over HTTP
        val resp = http.newCall(Request.Builder()
            .url("$baseUrl/api/v1/agent/tasks/$taskId?token=$token").get().build()).execute()
        if (!resp.isSuccessful) return
        val brief = JSONObject(resp.body!!.string())
        send(webSocket, "device.task.ack", JSONObject().put("task_id", taskId).put("state", "EXECUTING"))

        // run the signed adapter through the Rust core; the accessibility service
        // supplies the live node tree, the interpreter decides every action
        val appVersion = SincyAccessibilityService.installedVersion(brief.getString("app_package")) ?: "0"
        val resultJson = AdapterRuntime.runAdapter(
            brief.getString("adapter_yaml"),
            brief.getString("signature"),
            appVersion,
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
