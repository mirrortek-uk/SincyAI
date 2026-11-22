package com.sincy.agent.perception

import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.Image
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.os.Handler
import android.os.HandlerThread
import android.util.Base64
import android.view.Display
import java.io.ByteArrayOutputStream

/**
 * 屏幕截图 (视觉能力, DroidRun 式): 用户在 MainActivity 授权 MediaProjection 后,
 * 这里挂一个常驻 VirtualDisplay 到 ImageReader, 每次 capture() 读最新一帧,
 * 缩放到最大边 540px、JPEG q70、base64 —— 喂给云端 VLM。
 *
 * 未授权时 capture() 返回 null, agent 自动退回纯控件树模式。
 */
object ScreenCapture {

    private var projection: MediaProjection? = null
    private var reader: ImageReader? = null
    private var display: VirtualDisplay? = null
    private var thread: HandlerThread? = null
    private var handler: Handler? = null
    private var width = 0
    private var height = 0

    @Synchronized
    fun start(proj: MediaProjection, disp: Display, densityDpi: Int = 420) {
        stop()
        width = disp.width.coerceAtLeast(1)
        height = disp.height.coerceAtLeast(1)
        val t = HandlerThread("sincy-screencap").apply { start() }
        thread = t
        handler = Handler(t.looper)
        val r = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 3)
        reader = r
        display = proj.createVirtualDisplay(
            "sincy-capture", width, height, densityDpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            r.surface, null, handler,
        )
        projection = proj
        proj.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() { stop() }
        }, handler)
    }

    @Synchronized
    fun stop() {
        try { display?.release() } catch (_: Exception) {}
        try { reader?.close() } catch (_: Exception) {}
        try { projection?.stop() } catch (_: Exception) {}
        display = null; reader = null; projection = null
        thread?.quitSafely()
        thread = null; handler = null
    }

    val active: Boolean get() = projection != null && reader != null

    /**
     * 最新一帧截图 → 缩放 → JPEG base64。整个过程应在几十毫秒内完成。
     * 没授权或拿不到帧时返回 null。
     */
    @Synchronized
    fun captureBase64Jpeg(maxEdge: Int = 540, quality: Int = 70): String? {
        val r = reader ?: return null
        // 给 virtual display 一小段时间出帧
        var img: Image? = null
        repeat(6) {
            if (img == null) {
                img = try { r.acquireLatestImage() } catch (_: Exception) { null }
                if (img == null) Thread.sleep(25)
            }
        }
        val image = img ?: return null
        try {
            val planes = image.planes
            val buffer = planes[0].buffer
            val pixelStride = planes[0].pixelStride
            val rowStride = planes[0].rowStride
            val rowPadding = rowStride - pixelStride * width
            val bmp = Bitmap.createBitmap(
                width + rowPadding / pixelStride, height, Bitmap.Config.ARGB_8888,
            )
            bmp.copyPixelsFromBuffer(buffer)
            // 裁掉 padding 列
            val full = if (rowPadding > 0) Bitmap.createBitmap(bmp, 0, 0, width, height) else bmp
            if (bmp !== full) bmp.recycle()
            val scale = maxEdge.toFloat() / maxOf(full.width, full.height).toFloat()
            val scaled = if (scale < 1f) {
                Bitmap.createScaledBitmap(
                    full,
                    (full.width * scale).toInt().coerceAtLeast(1),
                    (full.height * scale).toInt().coerceAtLeast(1),
                    true,
                )
            } else full
            if (scaled !== full) full.recycle()
            val bos = ByteArrayOutputStream()
            scaled.compress(Bitmap.CompressFormat.JPEG, quality, bos)
            if (scaled !== full) scaled.recycle()
            return Base64.encodeToString(bos.toByteArray(), Base64.NO_WRAP)
        } catch (_: Exception) {
            return null
        } finally {
            try { image.close() } catch (_: Exception) {}
        }
    }
}
