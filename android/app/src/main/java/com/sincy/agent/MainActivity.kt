package com.sincy.agent

import android.content.Intent
import android.os.Bundle
import android.provider.Settings
import android.widget.Button
import android.widget.EditText
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.edit
import androidx.lifecycle.lifecycleScope
import android.Manifest
import android.content.pm.PackageManager
import com.sincy.agent.core.AgentForegroundService
import com.sincy.agent.perception.SincyAccessibilityService
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.security.MessageDigest

/**
 * Entry point. Pairing flow (WP-07 challenge-response):
 *  1. owner enters cloud URL + account email/password (same credentials as the console login);
 *  2. POST /devices/register/start -> one-time device_secret + nonce (secret shown once,
 *     only its hash is stored server-side);
 *  3. proof = sha256(sha256(secret) + nonce); POST /devices/register/complete -> device token;
 *  4. token is stored in EncryptedSharedPreferences (Android Keystore-backed);
 *  5. user enables the accessibility service in system settings, then the foreground
 *     service starts and holds the WSS control channel.
 */
class MainActivity : AppCompatActivity() {

    private val http = OkHttpClient()

    private val screenCapLauncher = registerForActivityResult(
        androidx.activity.result.contract.ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        if (result.resultCode == android.app.Activity.RESULT_OK && result.data != null) {
            // Android 14+: 必须经 mediaProjection 前台服务拿 projection, 直接在 Activity
            // 调 getMediaProjection 会 SecurityException 崩溃
            com.sincy.agent.core.MediaProjectionService.startWithResult(
                this,
                result.resultCode,
                result.data!!,
            )
            Toast.makeText(this, "正在启动录屏服务… 看通知栏", Toast.LENGTH_LONG).show()
        } else {
            Toast.makeText(this, "未授权录屏 — agent 只能用控件树(无视觉)", Toast.LENGTH_LONG).show()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // 打开 app 期间屏幕常亮
        window.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        // Minimal programmatic UI (a layout can replace this without touching logic)
        val root = android.widget.LinearLayout(this).apply {
            orientation = android.widget.LinearLayout.VERTICAL
            setPadding(48, 48, 48, 48)
        }
        val version = android.widget.TextView(this).apply {
            text = "Sincy Agent v${BuildConfig.VERSION_NAME}"
            textSize = 12f
        }
        val server = EditText(this).apply {
            hint = "Cloud URL"
            setText(BuildConfig.DEFAULT_SERVER)
        }
        val code = EditText(this).apply {
            hint = "配对码 (控制台生成的 4 位数字)"
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS
        }
        val pair = Button(this).apply { text = "Pair this device" }
        val openA11y = Button(this).apply { text = "Enable accessibility service" }
        val start = Button(this).apply { text = "Start agent" }
        val antilock = android.widget.CheckBox(this).apply {
            text = "防锁屏 (agent 运行时屏幕常亮, 需要系统设置授权)"
            isChecked = prefs().getBoolean("antilock", true)
            setOnCheckedChangeListener { _, checked ->
                prefs().edit { putBoolean("antilock", checked) }
                AgentForegroundService.setAntilock(checked)
            }
        }
        val grantSettings = Button(this).apply {
            text = "授权修改系统设置 (防锁屏需要)"
            setOnClickListener {
                if (!android.provider.Settings.System.canWrite(this@MainActivity)) {
                    startActivity(
                        Intent(
                            android.provider.Settings.ACTION_MANAGE_WRITE_SETTINGS,
                            android.net.Uri.parse("package:$packageName"),
                        ),
                    )
                } else {
                    Toast.makeText(this@MainActivity, "已授权 ✔", Toast.LENGTH_SHORT).show()
                }
            }
        }
        val grantScreen = Button(this).apply {
            text = "授权录屏 (视觉能力: agent 直接看屏幕截图)"
            setOnClickListener {
                val mpm = getSystemService(android.media.projection.MediaProjectionManager::class.java)
                screenCapLauncher.launch(mpm.createScreenCaptureIntent())
            }
        }
        root.addView(version); root.addView(server); root.addView(code)
        root.addView(pair); root.addView(openA11y); root.addView(start)
        root.addView(antilock); root.addView(grantSettings); root.addView(grantScreen)
        setContentView(root)

        pair.setOnClickListener {
            lifecycleScope.launch {
                try {
                    val existing = prefs().getString("device_token", null)
                    if (code.text.toString().isBlank() && !existing.isNullOrEmpty()) {
                        Toast.makeText(this@MainActivity, "已配对 (v${BuildConfig.VERSION_NAME})，如需重新配对请在控制台生成新码后再输入", Toast.LENGTH_LONG).show()
                        return@launch
                    }
                    val r = pairWithCode(server.text.toString(), code.text.toString())
                    prefs().edit {
                        putString("device_token", r.token)
                        putString("device_id", r.deviceId)
                        putString("agency_id", r.agencyId)
                        putString("cloud_url", r.baseUrl)
                    }
                    Toast.makeText(this@MainActivity, "配对成功 ✔ v${BuildConfig.VERSION_NAME}，设备 ${r.deviceId.take(8)}", Toast.LENGTH_LONG).show()
                } catch (e: Exception) {
                    Toast.makeText(this@MainActivity, "配对失败: ${e.message}", Toast.LENGTH_LONG).show()
                }
            }
        }
        openA11y.setOnClickListener { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
        start.setOnClickListener {
            val token = prefs().getString("device_token", null)
            if (token.isNullOrEmpty()) {
                Toast.makeText(this, "请先配对 (输入 4 位配对码)", Toast.LENGTH_LONG).show()
                return@setOnClickListener
            }
            // 服务地址以当前输入框为准
            prefs().edit().putString("cloud_url", server.text.toString().trim().ifBlank { "http://127.0.0.1:18081" }).apply()
            // Android 13+ 前台服务通知需要运行时权限
            if (android.os.Build.VERSION.SDK_INT >= 33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
            }
            if (!a11yEnabled()) {
                Toast.makeText(this, "建议先点上方按钮开启无障碍服务，否则 agent 看不到屏幕", Toast.LENGTH_LONG).show()
            }
            try {
                startAgentService()
                Toast.makeText(this, "Agent 已启动 — 连接状态看通知栏 (WSS ${server.text})", Toast.LENGTH_LONG).show()
            } catch (e: Exception) {
                Toast.makeText(this, "启动失败: ${e.message}", Toast.LENGTH_LONG).show()
            }
        }

        // 已配对过的设备每次打开 app 都自动拉起 agent 服务,
        // 免得服务被系统杀掉后手机"假在线" (任务一直 QUEUED 无人接手)
        if (!prefs().getString("device_token", null).isNullOrEmpty()) {
            try {
                startAgentService()
            } catch (_: Exception) {
            }
        }
    }

    private fun startAgentService() {
        if (android.os.Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        startForegroundService(Intent(this, AgentForegroundService::class.java))
    }

    override fun onResume() {
        super.onResume()
        // 从系统设置授权页回来后, 立即生效防锁屏
        if (prefs().getBoolean("antilock", true)) {
            AgentForegroundService.setAntilock(true)
        }
    }

    private fun prefs() = Prefs.get(this)

    private fun a11yEnabled(): Boolean {
        val enabled = Settings.Secure.getString(
            contentResolver, Settings.Secure.ENABLED_ACCESSIBILITY_SERVICES) ?: return false
        return enabled.split(':').any { it.contains("SincyAccessibilityService", ignoreCase = true) }
    }

    data class PairResult(val token: String, val deviceId: String, val agencyId: String, val baseUrl: String)

    /** 只凭配对码配对: POST /devices/pair (公开端点), 不需要账号密码。 */
    private suspend fun pairWithCode(baseUrl: String, code: String): PairResult = withContext(Dispatchers.IO) {
        val base = baseUrl.trim().ifBlank { "http://127.0.0.1:18081" }
        fun httpCall(url: String, body: String): okhttp3.Response {
            return try {
                http.newCall(Request.Builder().url(url).post(body.toRequestBody("application/json".toMediaType())).build()).execute()
            } catch (e: java.net.UnknownHostException) {
                throw IllegalStateException("DNS 解析失败: $url — 检查填写的地址")
            } catch (e: java.net.ConnectException) {
                throw IllegalStateException("连接被拒绝: $url — 电脑上服务没在跑? 先跑 bash /d/tmp/start-cloud-api-prod.sh; 手机到电脑先 adb reverse tcp:18081 tcp:18081")
            } catch (e: java.net.SocketTimeoutException) {
                throw IllegalStateException("连接超时: $url — 检查 USB/网络")
            } catch (e: javax.net.ssl.SSLException) {
                throw IllegalStateException("TLS/明文被拦截: $url — release 包禁明文, 本地请 adb reverse + http://127.0.0.1:18081, 云端用 https")
            } catch (e: java.io.IOException) {
                throw IllegalStateException("网络错误: ${e.message} — url=$url")
            }
        }
        if (code.isBlank()) throw IllegalStateException("请输入控制台生成的 4 位数字配对码")
        val body = JSONObject()
            .put("code", code.trim().uppercase())
            .put("platform", "android")
            .put("name", android.os.Build.MODEL)
            .toString()
        val resp = httpCall("$base/api/v1/devices/pair", body)
        if (!resp.isSuccessful) {
            val b = resp.body?.string() ?: ""
            throw IllegalStateException("配对失败 HTTP ${resp.code} @ $base: ${b.take(120)} — 码错误/过期/已用过, 请重新生成")
        }
        val j = JSONObject(resp.body!!.string())
        PairResult(j.getString("token"), j.getString("device_id"), j.optString("agency_id"), base)
    }

    private fun sha256(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
}
