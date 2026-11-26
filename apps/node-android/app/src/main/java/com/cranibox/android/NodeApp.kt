package com.cranibox.android

import android.app.Application
import android.content.Context
import android.os.SystemClock
import com.cranibox.android.api.NodeRepository
import com.cranibox.android.core.bridge.Bridge
import com.cranibox.android.core.bridge.FilePauseStore
import com.cranibox.android.core.helper.HandoffNonces
import com.cranibox.android.core.helper.NonceSlots
import com.cranibox.android.core.helper.NonceStore
import kotlinx.coroutines.flow.MutableStateFlow
import java.io.File

class NodeApp : Application() {
    val phone by lazy { AndroidPhone(this) }
    val bridge by lazy { Bridge(phone, FilePauseStore(File(filesDir, "computer-pause.json"))) }
    val nonces by lazy { HandoffNonces(PreferenceNonces(this), SystemClock::elapsedRealtime, System::currentTimeMillis) }
    val status = MutableStateFlow("节点尚未启动")
    val loginUrl = MutableStateFlow<String?>(null)
    val agentScreenVisible = MutableStateFlow(false)
    @Volatile var acceptHelpers = true
    val adb by lazy { PhoneSetup(this) }
    val repository by lazy { NodeRepository(this) }

    override fun onCreate() {
        super.onCreate()
        // Android's app uid has no traditional HOME; the Rust child receives explicit paths.
        File(filesDir, ".config/cranibox").mkdirs()
        // Load persisted emergency stop before a surviving helper can reattach via the provider.
        bridge.paused
    }
}

val Context.node: NodeApp get() = applicationContext as NodeApp

private class PreferenceNonces(context: Context) : NonceStore {
    private val prefs = context.getSharedPreferences("helper-nonces", Context.MODE_PRIVATE)
    override fun load() = NonceSlots(
        prefs.getString("resume", null), prefs.getString("launch", null), prefs.getLong("launchAt", 0),
        prefs.getString("usb", null), prefs.getLong("usbAt", 0),
    )

    override fun save(slots: NonceSlots) {
        check(prefs.edit().clear().putString("resume", slots.resume).putString("launch", slots.launch)
            .putLong("launchAt", slots.launchAt).putString("usb", slots.usb).putLong("usbAt", slots.usbAt).commit()) {
            "cannot save helper credentials"
        }
    }
}
