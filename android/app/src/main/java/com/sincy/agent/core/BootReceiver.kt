package com.sincy.agent.core

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.sincy.agent.Prefs

/**
 * 开机自启: 已配对设备在 BOOT_COMPLETED 后自动拉起 agent 服务,
 * 避免重启后手机离线导致任务一直 QUEUED。未配对时静默跳过。
 */
class BootReceiver : BroadcastReceiver() {

    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        val token = Prefs.get(context).getString("device_token", null)
        if (token.isNullOrEmpty()) return
        try {
            context.startForegroundService(Intent(context, AgentForegroundService::class.java))
        } catch (_: Exception) {
        }
    }
}
