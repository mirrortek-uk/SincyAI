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
        val server = EditText(this).apply {
            hint = "Cloud URL"
            setText(BuildConfig.DEFAULT_SERVER)
        }
        val email = EditText(this).apply { hint = "Account email"; setText(BuildConfig.DEFAULT_EMAIL) }
        val password = EditText(this).apply {
            hint = "Account password"
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
            setText(BuildConfig.DEFAULT_PASSWORD)
        }
        val pair = Button(this).apply { text = "Pair this device" }
        val openA11y = Button(this).apply { text = "Enable accessibility service" }
        val start = Button(this).apply { text = "Start agent" }
        root.addView(server); root.addView(email); root.addView(password)
        root.addView(pair); root.addView(openA11y); root.addView(start)
        setContentView(root)

        pair.setOnClickListener {
            lifecycleScope.launch {
                try {
                    val token = pairDevice(server.text.toString().ifBlank { "http://10.0.2.2:18080" }, email.text.toString(), password.text.toString())
                    prefs().edit { putString("device_token", token) }
                    Toast.makeText(this@MainActivity, "Device paired", Toast.LENGTH_LONG).show()
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

    private suspend fun pairDevice(baseUrl: String, email: String, password: String): String = withContext(Dispatchers.IO) {
        val base = baseUrl.trim().ifBlank { "http://127.0.0.1:18081" }
        fun httpCall(url: String, body: String, token: String? = null): okhttp3.Response {
            val rb = Request.Builder().url(url).post(body.toRequestBody("application/json".toMediaType()))
            if (token != null) rb.addHeader("Authorization", "Bearer $token")
            return try {
                http.newCall(rb.build()).execute()
            } catch (e: java.net.UnknownHostException) {
                throw IllegalStateException("DNS 解析失败: $url — 检查填写的地址是否正确")
            } catch (e: java.net.ConnectException) {
                throw IllegalStateException("连接被拒绝: $url — 电脑上服务没在跑? 本地生产库先跑 bash /d/tmp/start-cloud-api-prod.sh; 手机到电脑要先 adb reverse tcp:18081 tcp:18081")
            } catch (e: java.net.SocketTimeoutException) {
                throw IllegalStateException("连接超时: $url — 检查手机和电脑的网络/USB 连接")
            } catch (e: javax.net.ssl.SSLException) {
                throw IllegalStateException("TLS/明文被拦截: $url — release 包禁明文, 本地请用 adb reverse + http://127.0.0.1:18081, 云端请用 https")
            } catch (e: java.io.IOException) {
                throw IllegalStateException("网络错误: ${e.message} — url=$url")
            }
        }
        // step 1: 登录拿 owner JWT
        val loginResp = httpCall("$base/api/v1/auth/login", JSONObject().put("email", email).put("password", password).toString())
        if (!loginResp.isSuccessful) {
            val b = loginResp.body?.string() ?: ""
            throw IllegalStateException("登录失败 HTTP ${loginResp.code}: ${b.take(120)} — 检查用户名/密码(本地生产库: admin@sincy.dev)")
        }
        val userToken = JSONObject(loginResp.body!!.string()).getString("token")
        // step 2: register/start (受保护路由)
        val startResp = httpCall("$base/api/v1/devices/register/start",
            JSONObject().put("platform", "android").put("name", android.os.Build.MODEL).toString(), userToken)
        if (!startResp.isSuccessful) throw IllegalStateException("register/start 失败 HTTP ${startResp.code}: ${startResp.body?.string()?.take(120)}")
        val ch = JSONObject(startResp.body!!.string())
        val secret = ch.getString("device_secret")
        val nonce = ch.getString("nonce")
        val deviceId = ch.getString("device_id")
        // proof = sha256(sha256(secret) + nonce) — 与服务端校验一致
        val secretHash = sha256(secret.toByteArray())
        val proof = sha256((secretHash + nonce).toByteArray())
        // step 3: register/complete (公开路由)
        val completeResp = httpCall("$base/api/v1/devices/register/complete",
            JSONObject().put("device_id", deviceId).put("nonce", nonce).put("proof", proof).toString())
        if (!completeResp.isSuccessful) throw IllegalStateException("register/complete 失败 HTTP ${completeResp.code}: ${completeResp.body?.string()?.take(120)} — 可能已配对过或 nonce 过期, 重试即可")
        JSONObject(completeResp.body!!.string()).getString("token")
    }

    private fun sha256(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
}
