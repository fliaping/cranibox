package com.cranibox.android.core.bridge

import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt

data class KeyChord(val keyCode: Int, val metaState: Int)

/**
 * Keys the Agent may press on its own display. System keys (Home, Recents, Power, Volume) are left out:
 * they act on the person's main screen or the whole phone.
 */
object Keys {
    // android.view.KeyEvent constants; :core has no Android dependency.
    private const val META_CTRL = 0x1000 or 0x2000 // META_CTRL_ON | META_CTRL_LEFT_ON
    private val plain = mapOf(
        "back" to 4,
        "escape" to 111, "esc" to 111,
        "enter" to 66, "return" to 66, "kp_enter" to 66,
        "tab" to 61,
        "backspace" to 67,
        "delete" to 112, "del" to 112,
        "up" to 19, "down" to 20, "left" to 21, "right" to 22,
        "page_up" to 92, "pageup" to 92, "prior" to 92,
        "page_down" to 93, "pagedown" to 93, "next" to 93,
        "home" to 122, "end" to 123, // move the text cursor, not the system Home key
        "space" to 62,
    )
    private val withCtrl = mapOf("a" to 29, "c" to 31, "v" to 50, "x" to 52, "z" to 54)

    fun parse(text: String): KeyChord? {
        if (text.isEmpty() || text.length > 64) return null
        val parts = text.trim().lowercase().split('+').map(String::trim)
        if (parts.any(String::isEmpty)) return null
        return when (parts.size) {
            1 -> plain[parts[0]]?.let { KeyChord(it, 0) }
            2 -> if (parts[0] == "ctrl" || parts[0] == "control") withCtrl[parts[1]]?.let { KeyChord(it, META_CTRL) } else null
            else -> null
        }
    }
}

/** Touch gestures for the bridge's mouse-style actions. */
object Gestures {
    fun drag(from: IntArray, to: IntArray, durationMs: Long = 300): List<StrokePoint> = line(from[0], from[1], to[0], to[1], durationMs, 10)

    fun longPress(x: Int, y: Int, durationMs: Long = 600): List<StrokePoint> =
        listOf(StrokePoint(x, y, 0), StrokePoint(x, y, durationMs / 2), StrokePoint(x, y, durationMs))

    /**
     * A swipe that scrolls content in [direction]: "down" shows what is below, so the finger moves up.
     * Distance is the screen's short side × min(amount, 10) / 12, kept inside the display.
     */
    fun scroll(direction: String, amount: Int, at: IntArray?, width: Int, height: Int): List<StrokePoint> {
        require(width > 0 && height > 0) { "display size unknown" }
        val x = at?.get(0) ?: (width / 2)
        val y = at?.get(1) ?: (height / 2)
        val distance = min(width, height) * min(max(amount, 1), 10) / 12
        val (dx, dy) = when (direction) {
            "down" -> 0 to -distance
            "up" -> 0 to distance
            "right" -> -distance to 0
            "left" -> distance to 0
            else -> throw IllegalArgumentException("scroll_direction must be up, down, left or right")
        }
        val ex = (x + dx).coerceIn(0, width - 1)
        val ey = (y + dy).coerceIn(0, height - 1)
        return line(x, y, ex, ey, 250, 8)
    }

    private fun line(x0: Int, y0: Int, x1: Int, y1: Int, durationMs: Long, segments: Int): List<StrokePoint> =
        (0..segments).map { i ->
            val f = i.toDouble() / segments
            StrokePoint(
                (x0 + (x1 - x0) * f).roundToInt(),
                (y0 + (y1 - y0) * f).roundToInt(),
                (durationMs * f).roundToInt().toLong(),
            )
        }
}
