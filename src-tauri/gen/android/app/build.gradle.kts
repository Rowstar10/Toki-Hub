import java.util.Properties
import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

// Toki Hub's own version, from src-tauri/Cargo.toml: 0.3.0 is version code 3000, and each release
// counts up, so Android installs an update over the version before
val hubVersion = file("../../../Cargo.toml").readLines()
    .first { it.startsWith("version = ") }.substringAfter("\"").substringBefore("\"")
val hubVersionCode = hubVersion.split(".").map { it.toInt() }.let { (major, minor, patch) -> major * 1000000 + minor * 1000 + patch }

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 37
    namespace = "com.rowstar10.tokihub"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.rowstar10.tokihub"
        minSdk = 24
        targetSdk = 37
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", hubVersionCode.toString()).toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", hubVersion)
    }
    // Release builds are signed with Toki Hub's key when GitHub has it (the same key every time,
    // so updates install over the old version); without it, with the debug key, for testing.
    val keystore = System.getenv("ANDROID_KEYSTORE_FILE")
    signingConfigs {
        create("release") {
            if (keystore != null) {
                storeFile = file(keystore)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("ANDROID_KEY_ALIAS") ?: "toki"
                keyPassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
            }
        }
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
            packaging {
                jniLibs.keepDebugSymbols.add("*/arm64-v8a/*.so")
                jniLibs.keepDebugSymbols.add("*/armeabi-v7a/*.so")
                jniLibs.keepDebugSymbols.add("*/x86/*.so")
                jniLibs.keepDebugSymbols.add("*/x86_64/*.so")
            }
        }
        getByName("release") {
            signingConfig = signingConfigs.getByName(if (keystore != null) "release" else "debug")
            optimization {
               enable = true
            }
            proguardFiles(
                *fileTree(".") {
                  include("**/*.pro")
                  exclude("build/**")
                }.files.toTypedArray()
            )
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    buildFeatures {
        buildConfig = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget = JvmTarget.JVM_1_8
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = file("tauri.build.gradle.kts"))
