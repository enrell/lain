// The Android app (DESIGN-v2 §8.5, D-245..D-254). The Rust core is built
// outside Gradle by `scripts/build-rust.sh` (cargo-ndk + uniffi-bindgen):
// it writes the native libraries to build/rustJniLibs/<abi>/ and the
// Kotlin bindings to build/generated/uniffi/, which are added here.
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "io.github.enrell.lain"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.enrell.lain"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    packaging {
        jniLibs {
            // Keep the libraries uncompressed and page-aligned.
            useLegacyPackaging = false
        }
    }

    sourceSets.getByName("main") {
        kotlin.srcDir("build/generated/uniffi")
        jniLibs.srcDir("build/rustJniLibs")
    }

    testOptions {
        unitTests.all { test ->
            // The bindings smoke test loads the host build of lain-ffi
            // when it is given (CI); it is skipped otherwise.
            val hostLib = providers.gradleProperty("lainHostLib").orNull
            if (hostLib != null) {
                test.systemProperty("uniffi.component.lain_ffi.libraryOverride", hostLib)
            }
        }
    }
}


dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.media3.exoplayer)
    implementation(libs.media3.exoplayer.hls)
    implementation(libs.media3.ui)
    implementation(libs.kotlinx.coroutines.android)
    // UniFFI's Kotlin bindings call the Rust library through JNA.
    implementation(libs.jna) { artifact { type = "aar" } }

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    // The desktop JNA (with its host dispatch library) for JVM tests.
    testImplementation(libs.jna)
}
