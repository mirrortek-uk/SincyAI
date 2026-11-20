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
        val server = EditText(this).apply { hint = "Cloud URL (default http://10.0.2.2:18080)" }
        val email = EditText(this).apply { hint = "Account email" }
        val password = EditText(this).apply { hint = "Account password"; inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD }
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
        // login first -> owner JWT; register/start is a protected route
        val loginBody = JSONObject().put("email", email).put("password", password).toString()
            .toRequestBody("application/json".toMediaType())
        val loginResp = http.newCall(
            Request.Builder().url("$baseUrl/api/v1/auth/login").post(loginBody).build()
        ).execute()
        if (!loginResp.isSuccessful) throw IllegalStateException("login: ${loginResp.body?.string()}")
        val userToken = JSONObject(loginResp.body!!.string()).getString("token")
        val startBody = JSONObject()
            .put("platform", "android").put("name", android.os.Build.MODEL).toString()
            .toRequestBody("application/json".toMediaType())
        val startResp = http.newCall(
            Request.Builder().url("$baseUrl/api/v1/devices/register/start")
                .addHeader("Authorization", "Bearer $userToken").post(startBody).build()
        ).execute()
        if (!startResp.isSuccessful) throw IllegalStateException("start: ${startResp.body?.string()}")
        val ch = JSONObject(startResp.body!!.string())
        val secret = ch.getString("device_secret")
        val nonce = ch.getString("nonce")
        val deviceId = ch.getString("device_id")
        // proof = sha256(sha256(secret) + nonce) — matches the server-side verifier
        val secretHash = sha256(secret.toByteArray())
        val proof = sha256((secretHash + nonce).toByteArray())
        val completeBody = JSONObject().put("device_id", deviceId).put("nonce", nonce).put("proof", proof).toString()
            .toRequestBody("application/json".toMediaType())
        val doneResp = http.newCall(
            Request.Builder().url("$baseUrl/api/v1/devices/register/complete").post(completeBody).build()
        ).execute()
        if (!doneResp.isSuccessful) throw IllegalStateException("complete: ${doneResp.body?.string()}")
        JSONObject(doneResp.body!!.string()).getString("token")
    }

    private fun sha256(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }
}
