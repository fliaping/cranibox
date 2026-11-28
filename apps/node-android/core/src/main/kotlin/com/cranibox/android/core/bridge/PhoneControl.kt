package com.cranibox.android.core.bridge

/** One point of a single-finger stroke; [tMs] is relative to the first point and never decreases. */
data class StrokePoint(val x: Int, val y: Int, val tMs: Long)
data class PhoneTask(val id: Int, val packageName: String)
data class PhoneApp(val packageName: String, val label: String)

enum class OpenAppResult(val wire: String) {
    STARTED("started"),
    /** The person is using the app on the main screen; it is not moved away from them. */
    IN_USE_ON_MAIN_SCREEN("in_use_on_main_screen"),
    NOT_FOUND("not_found"),
    ERROR("error"),
}

data class PhoneStatus(
    val helperConnected: Boolean,
    val displayId: Int?,
    val width: Int,
    val height: Int,
    val trustedDisplay: Boolean,
)

class Png(val bytes: ByteArray, val width: Int, val height: Int)

/**
 * The Agent's phone screen, implemented by the app over the shell helper and its virtual display.
 * Calls may block and are never made on the main thread. Input only ever reaches the Agent's display.
 */
interface PhoneControl {
    /** Stops a running input sequence too; implementations must allow release/cancel events. */
    fun setPaused(paused: Boolean) {}
    fun status(): PhoneStatus
    /** Latest frame, optionally cropped to [left, top, right, bottom]; null before the first frame. */
    fun screenshot(region: IntArray? = null): Png?
    fun tap(x: Int, y: Int): Boolean
    /** One finger: the first point goes down, the last point goes up. */
    fun stroke(points: List<StrokePoint>): Boolean
    /** Android KeyEvent key code and meta state; sends down and up. */
    fun key(keyCode: Int, metaState: Int): Boolean
    /** Printable ASCII plus \n and \t, as key events. */
    fun typeAscii(text: String): Boolean
    /** Any text, through the clipboard and a paste key. */
    fun paste(text: String): Boolean
    fun tasks(): List<PhoneTask>
    fun focusTask(id: Int): Boolean
    fun apps(): List<PhoneApp>
    fun openApp(packageName: String): OpenAppResult
}
