// Compiles :app's Kotlin against android.jar on hosts without the Android SDK tools
// (arm64 Linux has no aapt2). Type checking only; nothing here is packaged.
plugins {
    alias(libs.plugins.kotlin.jvm)
}

kotlin { jvmToolchain(17) }

sourceSets {
    main {
        kotlin.srcDir("../app/src/main/java")
    }
}

dependencies {
    compileOnly(files(System.getenv("ANDROID_JAR")))
    implementation(project(":core"))
    implementation(libs.kotlinx.coroutines.android)
}
