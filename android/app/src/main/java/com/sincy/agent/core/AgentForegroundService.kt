package com.sincy.agent.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Intent
import android.os.IBinder
import androidx.core.app.NotificationCompat
import com.sincy.agent.R

/**
 * Long-running agent process. Owns the WSS control channel (net/AgentGateway)
 * and drives the adapter interpreter (adapter/AdapterRuntime) via the Rust
 * core (rust-core, JNI).
 *
 * Hard rules (Implementation Guide): never auto-solve challenge screens
 * (captcha/security) — report challenge.detected and park the task as
 * WAITING_USER; never forge device fingerprints; only run signed adapters
 * whose version_range covers the target app version.
 */
class AgentForegroundService : Service() {

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        val ch = NotificationChannel(CHANNEL, "Sincy Agent", NotificationManager.IMPORTANCE_LOW)
        nm.createNotificationChannel(ch)
        val n: Notification = NotificationCompat.Builder(this, CHANNEL)
            .setContentTitle("Sincy Agent")
            .setContentText("connected to cloud")
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .build()
        startForeground(NOTIF_ID, n)
        // TODO(WP-11): AgentGateway.connect(token) + heartbeat loop
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        const val CHANNEL = "sincy.agent"
        const val NOTIF_ID = 1
    }
}
