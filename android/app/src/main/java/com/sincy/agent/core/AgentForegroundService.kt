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
    private var wakeLock: android.os.PowerManager.WakeLock? = null
    private var savedScreenOffTimeout: Int = -1
    // 同一时刻只跑一个任务: gateway 会把多条 QUEUED 一次性派发, 串行执行避免
    // 两个 agentic 闭环同时抢屏幕/同时调 LLM 互相干扰
    private val taskMutex = kotlinx.coroutines.sync.Mutex()

    override fun onCreate() {
        super.onCreate()
        instance = this
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Sincy Agent", NotificationManager.IMPORTANCE_LOW))
        applyAntiLock()
        notify("启动中…")
        connect()
    }

    /**
     * 防锁屏: 服务存活期间 (1) 持有 PARTIAL_WAKE_LOCK 保证 CPU 不休眠 (WSS/解释器持续跑);
     * (2) 若用户开启了防锁屏 (默认开) 且已授予 WRITE_SETTINGS, 把系统屏幕休眠时间改成"永不",
     *     服务退出时恢复原值。没有 WRITE_SETTINGS 授权时静默跳过 (2), (1) 不受影响。
     */
    private fun applyAntiLock() {
        try {
            val pm = getSystemService(android.os.PowerManager::class.java)
            wakeLock = pm.newWakeLock(
                android.os.PowerManager.PARTIAL_WAKE_LOCK,
                "sincy:agent-control",
            ).apply { acquire() }
        } catch (_: Exception) {
        }
        val enabled = antilockRuntime ?: prefs().getBoolean("antilock", true)
        if (enabled) {
            try {
                if (android.provider.Settings.System.canWrite(this)) {
                    savedScreenOffTimeout = android.provider.Settings.System.getInt(
                        contentResolver,
                        android.provider.Settings.System.SCREEN_OFF_TIMEOUT,
                        30_000,
                    )
                    android.provider.Settings.System.putInt(
                        contentResolver,
                        android.provider.Settings.System.SCREEN_OFF_TIMEOUT,
                        Int.MAX_VALUE, // 永不休眠
                    )
                }
            } catch (_: Exception) {
            }
        }
    }

    /** 运行时切换防锁屏 (MainActivity 勾选框调用) */
    private fun setAntilockInstance(enabled: Boolean) {
        antilockRuntime = enabled
        if (enabled) {
            applyAntiLock()
        } else {
            releaseAntiLock()
        }
    }

    private fun releaseAntiLock() {
        try {
            wakeLock?.let { if (it.isHeld) it.release() }
        } catch (_: Exception) {
        }
        wakeLock = null
        if (savedScreenOffTimeout >= 0) {
            try {
                android.provider.Settings.System.putInt(
                    contentResolver,
                    android.provider.Settings.System.SCREEN_OFF_TIMEOUT,
                    savedScreenOffTimeout,
                )
            } catch (_: Exception) {
            }
            savedScreenOffTimeout = -1
        }
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
        taskMutex.lock()
        try {
            handleAssignLocked(webSocket, baseUrl, token, taskId)
        } finally {
            taskMutex.unlock()
        }
    }

    private suspend fun handleAssignLocked(webSocket: WebSocket, baseUrl: String, token: String, taskId: String) {
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
        releaseAntiLock()
        instance = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        const val CHANNEL = "sincy.agent"
        const val NOTIF_ID = 1

        /** 防锁屏开关的运行时镜像 (MainActivity 勾选框实时改这里, 服务立即生效) */
        @Volatile
        var antilockRuntime: Boolean? = null

        /** 当前运行的服务实例, 供 UI 勾选框实时切换防锁屏 */
        @Volatile
        var instance: AgentForegroundService? = null

        fun setAntilock(enabled: Boolean) {
            instance?.setAntilockInstance(enabled)
        }
    }
}
