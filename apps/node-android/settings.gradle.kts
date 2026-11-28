pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "cranibox-android"

include(":core")
// The Android module needs the Android SDK (and, for resources, an x86_64 or macOS aapt2).
// Without an SDK only the pure-JVM :core module is configured, so its tests run anywhere.
val hasSdk = file("local.properties").let { it.exists() && it.readText().contains("sdk.dir") } ||
    System.getenv("ANDROID_HOME") != null || System.getenv("ANDROID_SDK_ROOT") != null
if (hasSdk) include(":app")

// Without an SDK, the app's Kotlin can still be type-checked against a platform android.jar:
//   ANDROID_JAR=/path/to/platforms/android-35/android.jar ./gradlew :typecheck:compileKotlin
if (System.getenv("ANDROID_JAR") != null) include(":typecheck")
