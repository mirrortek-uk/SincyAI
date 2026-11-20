package com.sincy.agent

import android.content.Intent
import android.os.Bundle
import android.provider.Settings
import androidx.appcompat.app.AppCompatActivity

/**
 * Entry point. Registration with Cloud uses challenge-response (secret returned
 * once, only its hash stored server-side — sincy-device). Accessibility must be
 * granted manually by the user in system settings.
 */
class MainActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Minimal UI: guide the user to enable the accessibility service, then
        // start AgentForegroundService which opens the WSS control channel.
        startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
    }
}
