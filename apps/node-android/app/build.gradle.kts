plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "com.cranibox.android"
    compileSdk = 35
    buildToolsVersion = "35.0.0"

    defaultConfig {
        applicationId = "com.cranibox.node"
        minSdk = 31
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += "arm64-v8a" }
    }

    buildTypes {
        release {
            // The shell helper and reflection on hidden APIs need class names kept as written.
            isMinifyEnabled = false
        }
    }

    packaging {
        // libcranibox.so is an executable: it must be extracted to the native library
        // directory, the only app directory Android lets the app execute from.
        jniLibs.useLegacyPackaging = true
        jniLibs.keepDebugSymbols += "**/libcranibox.so"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

val verifyNative by tasks.registering {
    val binary = layout.projectDirectory.file("src/main/jniLibs/arm64-v8a/libcranibox.so")
    inputs.file(binary)
    doLast { check(binary.asFile.isFile) { "Build the Android node first: ./build-native.sh" } }
}
tasks.named("preBuild") { dependsOn(verifyNative) }

val legalAssets by tasks.registering(Copy::class) {
    from(rootProject.file("NOTICE"), rootProject.file("../../LICENSE"))
    into(layout.buildDirectory.dir("generated/legalAssets"))
}
android.sourceSets.getByName("main").assets.srcDir(layout.buildDirectory.dir("generated/legalAssets"))
tasks.named("preBuild") { dependsOn(legalAssets) }

dependencies {
    implementation(project(":core"))
    implementation(libs.kotlinx.coroutines.android)
}
