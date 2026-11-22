package com.sincy.agent.setup

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import com.sincy.agent.adapter.AdapterRuntime
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/**
 * 原生系统 op 执行器 (签名 system.* adapter 专用)。
 *
 * 安全约定:
 *  - 下载地址 / sha256 / 目标包名全部来自服务端签名的 adapter params, 本地不可改;
 *  - download_file 校验 SHA-256, 不匹配直接失败 (防篡改);
 *  - install_apk 只调起系统安装器弹窗 —— 人手点「安装」是 Android 硬要求, agent 不跳过;
 *  - verify_signer 用 PackageManager 校验安装后应用的签名证书指纹;
 *  - 账号密码永远不经 agent 之手 —— 安装完成后 launch 到登录页即上报 waiting_user。
 */
object SetupOps {

    private fun sha256Hex(bytes: ByteArray): String =
        MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) }

    private fun result(ok: Boolean, detail: String) {
        AdapterRuntime.lastOpResult = if (ok) detail else "ERR:$detail"
    }

    /** @return true 表示 op 接受并执行完成(或已启动), 详细结果写入 lastOpResult */
    fun execute(ctx: Context, op: JSONObject?): Boolean {
        if (op == null) return false
        return when (op.optString("op")) {
            "download_file" -> downloadFile(ctx, op.optString("url"), op.optString("sha256"))
            "install_apk" -> installApk(ctx)
            "wait_installed" -> waitInstalled(ctx, op.optString("package"), op.optLong("timeout_ms", 180_000))
            "verify_signer" -> verifySigner(ctx, op.optString("package"), op.optString("cert_sha256"))
            "launch_app" -> launchApp(ctx, op.optString("package"))
            "wait_ms" -> { Thread.sleep(op.optLong("ms", 0)); result(true, "ok"); true }
            else -> { result(false, "unknown op"); false }
        }
    }

    private fun downloadFile(ctx: Context, url: String, sha256: String): Boolean {
        return try {
            val dest = File(ctx.cacheDir, "setup_download.apk")
            val conn = URL(url).openConnection() as HttpURLConnection
            conn.connectTimeout = 30_000
            conn.readTimeout = 120_000
            conn.instanceFollowRedirects = true
            conn.connect()
            if (conn.responseCode !in 200..299) {
                result(false, "http ${conn.responseCode}")
                return false
            }
            conn.inputStream.use { input -> dest.outputStream().use { input.copyTo(it) } }
            conn.disconnect()
            val got = sha256Hex(dest.readBytes())
            if (!got.equals(sha256, ignoreCase = true)) {
                dest.delete()
                result(false, "sha256 mismatch got=$got")
                return false
            }
            result(true, "downloaded ${dest.length()} bytes sha256 ok")
            true
        } catch (e: Exception) {
            result(false, "download failed: ${e.message}")
            false
        }
    }

    private fun installApk(ctx: Context): Boolean {
        return try {
            val apk = File(ctx.cacheDir, "setup_download.apk")
            if (!apk.exists()) { result(false, "apk not downloaded"); return false }
            // 未知来源授权检查 (Android 8+): 没权限时打开系统设置页让人手开
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !ctx.packageManager.canRequestPackageInstalls()) {
                val intent = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                    Uri.parse("package:${ctx.packageName}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                ctx.startActivity(intent)
                result(false, "need user to allow install from this source")
                return false
            }
            val uri = FileProvider.getUriForFile(ctx, "${ctx.packageName}.fileprovider", apk)
            val intent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, "application/vnd.android.package-archive")
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            ctx.startActivity(intent)
            result(true, "install intent fired")
            true
        } catch (e: Exception) {
            result(false, "install intent failed: ${e.message}")
            false
        }
    }

    private fun waitInstalled(ctx: Context, packageName: String, timeoutMs: Long): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            try {
                ctx.packageManager.getPackageInfo(packageName, 0)
                result(true, "installed")
                return true
            } catch (_: PackageManager.NameNotFoundException) {
                Thread.sleep(1000)
            }
        }
        result(false, "timeout waiting for $packageName to be installed (user must tap 安装)")
        return false
    }

    @Suppress("DEPRECATION")
    private fun verifySigner(ctx: Context, packageName: String, certSha256: String): Boolean {
        try {
            val pm = ctx.packageManager
            val certs: List<ByteArray> = if (Build.VERSION.SDK_INT >= 28) {
                val info = pm.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES)
                info.signingInfo?.apkContentsSigners?.map { it.toByteArray() } ?: emptyList()
            } else {
                val info = pm.getPackageInfo(packageName, PackageManager.GET_SIGNATURES)
                info.signatures?.map { it.toByteArray() } ?: emptyList()
            }
            if (certs.isEmpty()) {
                result(false, "no signer certs")
                return false
            }
            val got = sha256Hex(certs[0])
            if (!got.equals(certSha256, ignoreCase = true)) {
                result(false, "signer cert mismatch got=$got")
                return false
            }
            result(true, "signer cert ok")
            return true
        } catch (e: Exception) {
            result(false, "verify signer failed: ${e.message}")
            return false
        }
    }

    private fun launchApp(ctx: Context, packageName: String): Boolean {
        return try {
            val intent = ctx.packageManager.getLaunchIntentForPackage(packageName)
            if (intent == null) { result(false, "no launch intent for $packageName"); return false }
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            ctx.startActivity(intent)
            result(true, "launched $packageName")
            true
        } catch (e: Exception) {
            result(false, "launch failed: ${e.message}")
            false
        }
    }
}
