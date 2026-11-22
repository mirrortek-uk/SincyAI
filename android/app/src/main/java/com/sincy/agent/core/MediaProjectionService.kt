package com.sincy.agent.core

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.IBinder
import android.view.WindowManager
import androidx.core.app.NotificationCompat
import com.sincy.agent.R
import com.sincy.agent.perception.ScreenCapture

/**
 * 录屏前台服务 (Android 14+ 硬性要求: getMediaProjection 必须在
 * FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION 的前台服务运行期间调用,
 * 否则 SecurityException 崩溃 —— 旧版本在 Activity 里直接调的写法在这会炸)。
 *
 * MainActivity 拿到系统授权结果后调 startWithResult() 启动本服务,
 * 服务先 startForeground(mediaProjection) 再 getMediaProjection, 然后把
 * projection 交给 ScreenCapture 挂常驻 VirtualDisplay。
 */
class MediaProjectionService : Service() {

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_START) {
            val resultCode = intent.getIntExtra(EXTRA_RESULT_CODE, Activity.RESULT_CANCELED)
            @Suppress("DEPRECATION")
            val data = intent.getParcelableExtra<Intent>(EXTRA_RESULT_DATA)
            if (resultCode != Activity.RESULT_OK || data == null) {
                stopSelf()
                return START_NOT_STICKY
            }
            val nm = getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, "Sincy 录屏(视觉)", NotificationManager.IMPORTANCE_LOW),
            )
            val notif = NotificationCompat.Builder(this, CHANNEL)
                .setContentTitle("Sincy Agent 视觉录屏中")
                .setContentText("agent 正在看屏幕截图做决策")
                .setSmallIcon(android.R.drawable.ic_menu_view)
                .setOngoing(true)
                .build()
            // 先挂前台服务(声明 mediaProjection 类型), 再拿 projection —— 顺序不能反
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(
                    NOTIF_ID,
                    notif,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION,
                )
            } else {
                startForeground(NOTIF_ID, notif)
            }
            val mpm = getSystemService(MediaProjectionManager::class.java)
            val proj: MediaProjection? = try {
                mpm.getMediaProjection(resultCode, data)
            } catch (e: Exception) {
                android.util.Log.e("MediaProjectionService", "getMediaProjection failed: ${e.message}")
                null
            }
            if (proj == null) {
                stopSelf()
                return START_NOT_STICKY
            }
            proj.registerCallback(object : MediaProjection.Callback() {
                override fun onStop() {
                    ScreenCapture.stop()
                    stopSelf()
                }
            }, null)
            val disp = (getSystemService(Context.WINDOW_SERVICE) as WindowManager).defaultDisplay
            ScreenCapture.start(proj, disp, resources.displayMetrics.densityDpi)
            android.util.Log.i("MediaProjectionService", "screen capture active: ${ScreenCapture.active}")
        }
        return START_STICKY
    }

    override fun onDestroy() {
        ScreenCapture.stop()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        const val CHANNEL = "sincy.screencap"
        const val NOTIF_ID = 2
        const val ACTION_START = "com.sincy.agent.SCREEN_CAP_START"
        const val EXTRA_RESULT_CODE = "result_code"
        const val EXTRA_RESULT_DATA = "result_data"

        fun startWithResult(ctx: Context, resultCode: Int, data: Intent) {
            val i = Intent(ctx, MediaProjectionService::class.java).apply {
                action = ACTION_START
                putExtra(EXTRA_RESULT_CODE, resultCode)
                putExtra(EXTRA_RESULT_DATA, data)
            }
            ctx.startForegroundService(i)
        }
    }
}
