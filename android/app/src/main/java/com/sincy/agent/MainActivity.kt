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
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import com.sincy.agent.core.AgentForegroundService
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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
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
            hint = "配对码 (控制台生成的 8 位码)"
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_FLAG_CAP_CHARACTERS
        }
        val pair = Button(this).apply { text = "Pair this device" }
        val openA11y = Button(this).apply { text = "Enable accessibility service" }
        val start = Button(this).apply { text = "Start agent" }
        root.addView(version); root.addView(server); root.addView(code)
        root.addView(pair); root.addView(openA11y); root.addView(start)
        setContentView(root)

        pair.setOnClickListener {
            lifecycleScope.launch {
                try {
                    val token = pairWithCode(server.text.toString(), code.text.toString())
                    prefs().edit { putString("device_token", token) }
                    Toast.makeText(this@MainActivity, "Device paired ✔ v${BuildConfig.VERSION_NAME}", Toast.LENGTH_LONG).show()
                } catch (e: Exception) {
                    Toast.makeText(this@MainActivity, "Pairing failed: ${e.message}", Toast.LENGTH_LONG).show()
                }
            }
        }
        openA11y.setOnClickListener { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
        start.setOnClickListener {
            val token = prefs().getString("device_token", null)
            if (token.isNullOrEmpty()) {
                Toast.makeText(this, "Pair first", Toast.LENGTH_LONG).show()
            } else {
                startForegroundService(Intent(this, AgentForegroundService::class.java))
            }
        }
    }

    private fun prefs() = EncryptedSharedPreferences.create(
        this,
        "sincy_agent_prefs",
        MasterKey.Builder(this).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )

    /** 只凭配对码配对: POST /devices/pair (公开端点), 不需要账号密码。 */
    private suspend fun pairWithCode(baseUrl: String, code: String): String = withContext(Dispatchers.IO) {
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
        if (code.isBlank()) throw IllegalStateException("请输入控制台生成的 8 位配对码")
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
        JSONObject(resp.body!!.string()).getString("token")
    }

    private fun sha256(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
}
