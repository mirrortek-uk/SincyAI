import java.util.Properties

plugins {
    id("com.android.application")
    kotlin("android")
    kotlin("plugin.serialization")
}

android {
    namespace = "com.sincy.agent"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.sincy.agent"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"

        // Rust core is built by cargo-ndk and loaded via JNI; see rust-core/
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a")
        }

        // 配对界面预填默认值: 构建期从环境变量注入, 代码仓库里不落任何密码.
        // 本地生产包构建脚本会从 /d/tmp/sincy-prod.env 读取; 不设置则为空, 手工填写.
        buildConfigField("String", "DEFAULT_SERVER", "\"${System.getenv("SINCY_AGENT_DEFAULT_SERVER") ?: "http://127.0.0.1:18081"}\"")
        buildConfigField("String", "DEFAULT_EMAIL", "\"${System.getenv("SINCY_AGENT_DEFAULT_EMAIL") ?: ""}\"")
        buildConfigField("String", "DEFAULT_PASSWORD", "\"${System.getenv("SINCY_AGENT_DEFAULT_PASSWORD") ?: ""}\"")
    }

    // 正式签名: android/key.properties (gitignored) -> keys/sincy-release.jks
    // 本地没有 key.properties 时回退 debug 签名, 便于纯本地验证
    signingConfigs {
        create("release") {
            val kp = File(rootDir, "key.properties")
            if (kp.exists()) {
                val props = Properties().apply { kp.inputStream().use { load(it) } }
                storeFile = File(rootDir, props.getProperty("storeFile"))
                storePassword = props.getProperty("storePassword")
                keyAlias = props.getProperty("keyAlias")
                keyPassword = props.getProperty("keyPassword")
            }
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            val kp = File(rootDir, "key.properties")
            signingConfig = if (kp.exists()) signingConfigs.getByName("release") else signingConfigs.getByName("debug")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
    buildFeatures { buildConfig = true }
}

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    implementation("com.google.android.material:material:1.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    // WSS control channel to cloud-api /api/v1/agent/connect (ADR-005)
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    // device token at rest: Keystore-backed encrypted prefs
    implementation("androidx.security:security-crypto:1.1.0-alpha06")
}
