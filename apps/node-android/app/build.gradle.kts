plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
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
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildTypes {
        release {
            // The shell helper and reflection on hidden APIs need class names kept as written.
            isMinifyEnabled = false
        }
    }

    buildFeatures { compose = true }

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
// Compile and unit tests do not need the native node; packaging and lint-on-APK do.
listOf("mergeDebugNativeLibs", "mergeReleaseNativeLibs", "assembleDebug", "assembleRelease", "lintDebug", "lintRelease").forEach { name ->
    tasks.matching { it.name == name }.configureEach { dependsOn(verifyNative) }
}

val legalAssets by tasks.registering(Copy::class) {
    from(rootProject.file("NOTICE"), rootProject.file("../../LICENSE"))
    into(layout.buildDirectory.dir("generated/legalAssets"))
}
android.sourceSets.getByName("main").assets.srcDir(layout.buildDirectory.dir("generated/legalAssets"))
tasks.named("preBuild") { dependsOn(legalAssets) }

dependencies {
    implementation(project(":core"))
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.okhttp)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons.extended)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.camera.core)
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.view)
    implementation(libs.mlkit.barcode.scanning)
    debugImplementation(libs.androidx.compose.ui.tooling)
    testImplementation(libs.junit)
    testImplementation(libs.kotlin.test)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.kotlinx.serialization.json)
}
