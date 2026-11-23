package com.cranibox.android.shell

import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.content.AttributionSource
import android.os.Binder
import android.os.Bundle
import android.os.IBinder
import android.os.Looper
import android.os.Process
import com.cranibox.android.core.helper.HelperHandoff
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/** Loaded from the signed APK by app_process, running as the ADB shell uid. */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi")
object HelperMain {
    @JvmStatic
    fun main(args: Array<String>) {
        try { run(args) }
        catch (error: Throwable) {
            android.util.Log.e("CraniboxShell", "Phone helper startup failed", error)
            throw error
        }
    }

    private fun run(args: Array<String>) {
        check(Process.myUid() == HelperHandoff.SHELL_UID)
        require(args.size == 3)
        val (pkg, uidText, pin) = args
        val appUid = uidText.toInt()
        var nonce = System.`in`.bufferedReader().readLine()?.takeIf { it.length == 64 } ?: return
        Looper.prepareMainLooper()
        val thread = Class.forName("android.app.ActivityThread")
        val activityThread = thread.getDeclaredMethod("systemMain").invoke(null)
        val system = thread.getDeclaredMethod("getSystemContext").invoke(activityThread) as Context
        // createPackageContext inherits the system context's "android" operation package.
        // Build a root application context from Shell's LoadedApk so provider calls and
        // display operations carry the package that actually belongs to our shell uid.
        val shellPackage = system.createPackageContext("com.android.shell", Context.CONTEXT_IGNORE_SECURITY)
        val contextImpl = Class.forName("android.app.ContextImpl")
        val loadedApk = contextImpl.getDeclaredField("mPackageInfo").apply { isAccessible = true }.get(shellPackage)
        val context = contextImpl.getDeclaredMethod(
            "createAppContext", thread, Class.forName("android.app.LoadedApk"),
        ).apply { isAccessible = true }.invoke(null, activityThread, loadedApk) as Context
        check(context.opPackageName == "com.android.shell") { "helper context has incorrect calling package" }
        val pm = context.packageManager
        @Suppress("DEPRECATION")
        fun validInstall(): Boolean {
            val info = pm.getPackageInfo(pkg, PackageManager.GET_SIGNING_CERTIFICATES)
            return info.applicationInfo?.uid == appUid && HelperHandoff.certPin(
                info.signingInfo?.apkContentsSigners?.map { it.toByteArray() },
            ) == pin
        }
        check(validInstall()) { "app signature or uid changed" }
        @Suppress("DEPRECATION")
        val apk = pm.getApplicationInfo(pkg, 0).sourceDir
        val shell = ShellService(context, appUid, apk)
        // Keep Binder calls available while the main looper handles display callbacks.
        Thread({
            try {
                while (validInstall()) {
                    val extras = Bundle().apply {
                        putString(HelperHandoff.KEY_NONCE, nonce)
                        putBinder(HelperHandoff.KEY_BINDER, shell)
                        putInt(HelperHandoff.KEY_UID, Process.myUid())
                        putInt(HelperHandoff.KEY_PID, Process.myPid())
                        putString(HelperHandoff.KEY_APK, apk)
                    }
                    val answer = handoff(context, HelperHandoff.authority(pkg), appUid / 100000, extras) ?: break
                    if (!answer.getBoolean(HelperHandoff.KEY_ACCEPTED)) {
                        android.util.Log.w("CraniboxShell", "App rejected helper: ${answer.getString(HelperHandoff.KEY_REASON)}")
                        break
                    }
                    nonce = answer.getString(HelperHandoff.KEY_NEXT) ?: break
                    val life = answer.getBinder(HelperHandoff.KEY_TOKEN) ?: break
                    val disconnected = CountDownLatch(1)
                    val death = IBinder.DeathRecipient { disconnected.countDown() }
                    try {
                        life.linkToDeath(death, 0)
                        while (!disconnected.await(30, TimeUnit.SECONDS)) {
                            if (!validInstall()) return@Thread
                        }
                    } finally {
                        life.unlinkToDeath(death, 0)
                        shell.releaseDisplay()
                    }
                    Thread.sleep(1_000)
                }
            } catch (error: Exception) {
                android.util.Log.e("CraniboxShell", "Phone helper handoff failed", error)
            } finally {
                shell.releaseDisplay()
                // An APK update, rejected handoff or explicit exit must not leave a shell helper alive.
                Process.killProcess(Process.myPid())
            }
        }, "cranibox-helper-handoff").start()
        Looper.loop()
    }

    /** app_process has no registered ApplicationThread; use the platform shell content path. */
    private fun handoff(context: Context, authority: String, userId: Int, extras: Bundle): Bundle? {
        val manager = Class.forName("android.app.ActivityManager").getDeclaredMethod("getService").invoke(null)
        val api = Class.forName("android.app.IActivityManager")
        val token = Binder()
        val holder = api.getMethod(
            "getContentProviderExternal", String::class.java, Int::class.javaPrimitiveType,
            IBinder::class.java, String::class.java,
        ).invoke(manager, authority, userId, token, "cranibox-helper") ?: error("helper provider unavailable")
        try {
            val provider = holder.javaClass.getField("provider").get(holder) ?: error("helper provider unavailable")
            return Class.forName("android.content.IContentProvider").getMethod(
                "call", AttributionSource::class.java, String::class.java, String::class.java,
                String::class.java, Bundle::class.java,
            ).invoke(provider, context.attributionSource, authority, HelperHandoff.METHOD_ATTACH, null, extras) as Bundle?
        } finally {
            api.getMethod(
                "removeContentProviderExternalAsUser", String::class.java, IBinder::class.java, Int::class.javaPrimitiveType,
            ).invoke(manager, authority, token, userId)
        }
    }
}
