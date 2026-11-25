import java.util.Properties
import java.util.Date
import java.text.SimpleDateFormat

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
        // 版本名带构建时间戳, 装到手机后一眼能看出是哪一版包
        versionName = "0.1.0-" + SimpleDateFormat("yyyyMMdd-HHmmss").format(Date())

        // Rust core is built by cargo-ndk and loaded via JNI; see rust-core/
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a")
        }

        // 配对界面只预填服务地址 (构建期 -PsincyAgentDefaultServer 注入);
        // 账号密码不出现在包里 — 配对改为"控制台生成一次性配对码, 手机只输入码".
        // 优先 -PsincyAgentDefault* (gradle daemon 不刷新 env, -P 每次构建都生效), 其次环境变量
        val defServer = (project.findProperty("sincyAgentDefaultServer") as String?)
            ?: System.getenv("SINCY_AGENT_DEFAULT_SERVER") ?: "http://127.0.0.1:18081"
        buildConfigField("String", "DEFAULT_SERVER", "\"$defServer\"")
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

// Rust core (libsincy_agent_core.so) 交叉编译: cargo-ndk -> app/src/main/jniLibs,
// 由 preBuild 触发, 保证打包进 APK。缺 NDK / cargo-ndk 时跳过并告警 (纯 Kotlin 壳仍可构建),
// CI 的 android-agent job 会安装 NDK + cargo-ndk 并校验 .so 产物。
val rustCoreDir = File(rootDir, "rust-core")
val jniLibsDir = File(rootDir, "app/src/main/jniLibs")
val ndkHome: String? = System.getenv("ANDROID_NDK_HOME")
    ?: runCatching {
        val props = Properties()
        File(rootDir, "local.properties").inputStream().use { props.load(it) }
        props.getProperty("ndk.dir")
            ?: props.getProperty("sdk.dir")?.let { sdk ->
                File(sdk, "ndk").listFiles()?.filter { it.isDirectory }?.maxByOrNull { it.name }?.absolutePath
            }
    }.getOrNull()

tasks.register("buildRustCore") {
    group = "build"
    description = "Cross-compile the Rust agent core into app/src/main/jniLibs via cargo-ndk"
    inputs.dir(File(rustCoreDir, "src"))
    inputs.file(File(rustCoreDir, "Cargo.toml"))
    outputs.dir(jniLibsDir)
    onlyIf {
        if (!File(rustCoreDir, "Cargo.toml").exists()) return@onlyIf false
        if (ndkHome == null) {
            logger.warn("buildRustCore skipped: ANDROID_NDK_HOME / local.properties ndk.dir not set")
            return@onlyIf false
        }
        val ok = runCatching {
            ProcessBuilder("cargo", "ndk", "--version")
                .redirectErrorStream(true).start().waitFor() == 0
        }.getOrDefault(false)
        if (!ok) logger.warn("buildRustCore skipped: cargo-ndk not installed (cargo install cargo-ndk)")
        ok
    }
    doLast {
        val p = ProcessBuilder(
            "cargo", "ndk", "-t", "arm64-v8a", "-t", "armeabi-v7a",
            "-o", jniLibsDir.absolutePath, "build", "--release"
        )
            .directory(rustCoreDir)
            .apply { environment()["ANDROID_NDK_HOME"] = ndkHome!! }
            .redirectErrorStream(true)
            .start()
        val out = p.inputStream.bufferedReader().readText()
        if (p.waitFor() != 0) {
            // 构建失败但已有预编译 .so 时降级告警继续 (本地增量开发不被卡住);
            // 一个 .so 都没有时必须失败, 否则 APK 会缺 native core, 手机上直接 UnsatisfiedLinkError
            val prebuilt = File(jniLibsDir, "arm64-v8a/libsincy_agent_core.so")
            if (prebuilt.exists() && prebuilt.length() > 0L) {
                logger.warn("buildRustCore: cargo-ndk failed, falling back to prebuilt $prebuilt\n$out")
            } else {
                throw GradleException("cargo-ndk failed and no prebuilt .so in $jniLibsDir:\n$out")
            }
        }
        // 产物校验: arm64 .so 必须存在 (armeabi-v7a 尽力而为)
        val so = File(jniLibsDir, "arm64-v8a/libsincy_agent_core.so")
        if (!so.exists() || so.length() == 0L) {
            throw GradleException("rust core .so missing after build: $so")
        }
        logger.lifecycle("buildRustCore: libsincy_agent_core.so present at $so")
    }
}

tasks.named("preBuild") { dependsOn("buildRustCore") }

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
