plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

/** The version of rustls-platform-verifier's Kotlin half must match the Rust crate in Cargo.lock. */
val rustlsVerifierVersion: String = run {
    val lines = rootDir.resolve("../../Cargo.lock").readLines()
    val at = lines.indexOfFirst { it.trim() == "name = \"rustls-platform-verifier-android\"" }
    require(at >= 0) { "rustls-platform-verifier-android is not in Cargo.lock" }
    lines[at + 1].substringAfter('"').substringBefore('"')
}

android {
    namespace = "nl.merel.app"
    compileSdk = 35

    defaultConfig {
        applicationId = "nl.merel.app"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    buildFeatures { compose = true }
}

kotlin { jvmToolchain(17) }

dependencies {
    val bom = platform("androidx.compose:compose-bom:2025.05.00")
    implementation(bom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.0")
    implementation("androidx.core:core-ktx:1.16.0")
    // UniFFI's Kotlin bindings call the Rust core through JNA.
    implementation("net.java.dev.jna:jna:5.17.0@aar")
    implementation("rustls:rustls-platform-verifier:$rustlsVerifierVersion")
}
