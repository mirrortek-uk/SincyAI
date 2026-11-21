package com.sincy.agent

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey

/**
 * 全 app 共用的加密 SharedPreferences（Android Keystore 背书)。
 * device_token / cloud_url / device_id / agency_id 只存在这里，
 * MainActivity 和 AgentForegroundService 必须用同一个实例读写。
 */
object Prefs {
    private const val FILE = "sincy_agent_prefs"

    fun get(ctx: Context): SharedPreferences = EncryptedSharedPreferences.create(
        ctx,
        FILE,
        MasterKey.Builder(ctx).setKeyScheme(MasterKey.KeyScheme.AES256_GCM).build(),
        EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
        EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
    )
}
