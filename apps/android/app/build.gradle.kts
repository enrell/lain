// The Android app (DESIGN-v2 §8.5, D-245..D-254). The Rust core is built
// outside Gradle by `scripts/build-rust.sh` (cargo-ndk + uniffi-bindgen):
// it writes the native libraries to build/rustJniLibs/<abi>/ and the
// Kotlin bindings to generated/uniffi/ (git-ignored), added here.
plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// Release identity (D-278), all from the environment so nothing secret
// is ever in the tree: the version from the release tag, and the signing
// key from a keystore file. Without a keystore the release APK is signed
// with the debug key, so a local build still installs over a debug one.
fun env(name: String): String? = providers.environmentVariable(name).orNull?.takeIf { it.isNotBlank() }
val releaseKeystore = env("LAIN_ANDROID_KEYSTORE")

// `LAIN_VERSION` is the release tag (`v0.3.1`); the version code follows
// from it as MAJOR * 1_000_000 + MINOR * 1_000 + PATCH (3001), so it is the
// same for every build of a tag and grows with every release. A suffix
// (`-rc.1`) does not count. `LAIN_ANDROID_VERSION_CODE` overrides it.
val lainVersion = env("LAIN_VERSION")?.removePrefix("v") ?: "0.3.1"
val lainVersionCode = env("LAIN_ANDROID_VERSION_CODE")?.let {
    it.toIntOrNull() ?: throw GradleException("LAIN_ANDROID_VERSION_CODE is not a number: $it")
} ?: run {
    val parts = lainVersion.substringBefore('-').substringBefore('+').split('.')
    val n = parts.mapNotNull { it.toIntOrNull()?.takeIf { v -> v in 0..999 } }
    if (parts.size != 3 || n.size != 3) {
        throw GradleException("LAIN_VERSION must be MAJOR.MINOR.PATCH, each below 1000: $lainVersion")
    }
    n[0] * 1_000_000 + n[1] * 1_000 + n[2]
}

android {
    namespace = "io.github.enrell.lain"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.enrell.lain"
        minSdk = 26
        targetSdk = 36
        versionCode = lainVersionCode
        versionName = lainVersion
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        releaseKeystore?.let { keystore ->
            create("release") {
                storeFile = file(keystore)
                storePassword = env("LAIN_ANDROID_KEYSTORE_PASSWORD")
                keyAlias = env("LAIN_ANDROID_KEY_ALIAS")
                keyPassword = env("LAIN_ANDROID_KEY_PASSWORD") ?: env("LAIN_ANDROID_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            // The build to put on a phone: optimized and shrunk (R8), and not
            // debuggable, which is what makes Compose scroll and start fast.
            // Signed with the release key when one is given, else with the
            // debug key (above).
            isMinifyEnabled = true
            isShrinkResources = true
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
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

    // The Anime4K shader packs are the web player's own files: one copy in
    // the repository, packaged here as assets (its TypeScript index is not).
    androidResources {
        ignoreAssetsPattern = "!.svn:!.git:!.ds_store:!*.scc:.*:!CVS:!thumbs.db:!picasa.ini:!*~:!*.ts"
    }

    sourceSets.getByName("main") {
        assets.srcDir("../../../web/src/lib/player/webgl/packs")
        java.srcDir("generated/uniffi")
        kotlin.srcDir("generated/uniffi")
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
    // Video effects (the Anime4K shaders) run in Media3's frame processor.
    implementation(libs.media3.effect)
    implementation(libs.kotlinx.coroutines.android)
    // UniFFI's Kotlin bindings call the Rust library through JNA.
    implementation(libs.jna) { artifact { type = "aar" } }

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    // The desktop JNA (with its host dispatch library) for JVM tests.
    testImplementation(libs.jna)
}

// AGP 9's built-in Kotlin compiles the variant's kotlin sources; register
// the generated bindings there too, so they are compiled with the app.
androidComponents {
    onVariants { variant ->
        val dir = layout.projectDirectory.dir("generated/uniffi").asFile.absolutePath
        variant.sources.kotlin?.addStaticSourceDirectory(dir)
    }
}
