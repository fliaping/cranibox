package com.cranibox.android.core.bridge

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject
import java.io.File
import java.security.SecureRandom
import java.time.Instant
import java.util.Base64
import kotlin.math.roundToInt

data class PauseState(val paused: Boolean, val reason: String = "", val changedAt: String = "")

/** Persists the emergency stop so it survives app and phone restarts. */
interface PauseStore {
    fun load(): PauseState
    fun save(state: PauseState)
}

class FilePauseStore(private val file: File) : PauseStore {
    override fun load(): PauseState {
        if (!file.exists()) return PauseState(false)
        return runCatching {
        val o = Json.parseToJsonElement(file.readText()).jsonObject
        PauseState(
            requireNotNull(o["paused"]?.jsonPrimitive?.booleanOrNull),
            o["reason"]?.jsonPrimitive?.contentOrNull ?: "",
            o["changed_at"]?.jsonPrimitive?.contentOrNull ?: "",
        )
        }.getOrDefault(PauseState(true, "pause state unreadable; resume on the phone"))
    }

    override fun save(state: PauseState) {
        file.parentFile?.mkdirs()
        val tmp = File(file.parentFile, ".${file.name}.tmp")
        tmp.writeText(
            buildJsonObject {
                put("paused", state.paused); put("reason", state.reason); put("changed_at", state.changedAt)
            }.toString(),
        )
        check(tmp.renameTo(file)) { "cannot save the pause state" }
    }
}

object BridgeToken {
    private val rng = SecureRandom()
    fun generate(): String = Base64.getUrlEncoder().withoutPadding().encodeToString(ByteArray(32).also(rng::nextBytes))
}

class BridgeResponse(val status: Int, val body: JsonObject)

/** Maps an expected failure to the HTTP status the Rust caller and agent-desktop-bridge use. */
private class BridgeError(val status: Int, message: String) : Exception(message)

private fun bad(message: String): Nothing = throw BridgeError(400, message)

/**
 * The phone's side of the Computer Use bridge protocol (`POST /v1/action`), matching agent-desktop-bridge:
 * owner actions (status, emergency_stop, resume) plus input and screen actions on the Agent's display.
 */
class Bridge(
    private val control: PhoneControl,
    private val pause: PauseStore,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    private val lock = Any()
    private val actionLock = Any()
    private var state = pause.load()
    private val sessions = LinkedHashMap<String, Pair<String, Long>>()
    private var last: IntArray? = null

    init { control.setPaused(state.paused) }

    val paused: Boolean get() = synchronized(lock) { state.paused }

    fun emergencyStop(reason: String): PauseState = setPaused(true, reason.ifBlank { "emergency stop" }.take(200))
    fun resume(): PauseState = setPaused(false, "")

    private fun setPaused(paused: Boolean, reason: String): PauseState = synchronized(lock) {
        PauseState(paused, reason, Instant.ofEpochMilli(clock()).toString()).also {
            // Stop before disk I/O, even if persisting the emergency stop fails.
            if (paused) { state = it; control.setPaused(true) }
            pause.save(it)
            state = it
            if (!paused) control.setPaused(false)
        }
    }

    fun execute(body: JsonObject, session: String): BridgeResponse = try {
        BridgeResponse(200, handle(body, session))
    } catch (e: BridgeError) {
        BridgeResponse(e.status, buildJsonObject {
            put("error", e.message ?: "error")
            if (e.status == 423) put("paused", true)
        })
    } catch (e: IllegalArgumentException) {
        BridgeResponse(400, buildJsonObject { put("error", (e.message ?: "invalid request").take(200)) })
    } catch (_: Exception) {
        BridgeResponse(503, buildJsonObject { put("error", "the phone could not complete the action") })
    }

    fun status(): JsonObject {
        val s = control.status()
        val ready = s.helperConnected && s.displayId != null
        val now = clock()
        val (pause, active) = synchronized(lock) {
            sessions.entries.removeIf { now - it.value.second > SESSION_TTL_MS }
            state to sessions.map { (id, v) -> Triple(id, v.first, (now - v.second) / 100 / 10.0) }
        }
        return buildJsonObject {
            put("service", "cranibox-android-bridge")
            put("platform", "android")
            put("state", if (pause.paused) "paused" else if (ready) "ready" else "backend-unavailable")
            put("paused", pause.paused)
            put("pause_reason", pause.reason)
            put("backend_available", ready)
            putJsonObject("resolution") { put("width", s.width); put("height", s.height) }
            put("display_id", s.displayId?.let(::JsonPrimitive) ?: JsonNull)
            putJsonObject("helper") { put("connected", s.helperConnected); put("trusted_display", s.trustedDisplay) }
            putJsonArray("active_sessions") {
                active.forEach { (id, action, idle) -> add(buildJsonObject { put("id", id); put("last_action", action); put("idle_seconds", idle) }) }
            }
            putJsonArray("capabilities") { CAPABILITIES.forEach { add(JsonPrimitive(it)) } }
            putJsonArray("coordinate_spaces") { add(JsonPrimitive("pixels")); add(JsonPrimitive("normalized")) }
        }
    }

    private fun handle(body: JsonObject, session: String): JsonObject {
        val action = body.string("action") ?: bad("action is required")
        record(session, action)
        when (action) {
            "status" -> return status()
            "emergency_stop" -> return ok(emergencyStop(body.string("reason") ?: ""))
            "resume" -> return ok(resume())
        }
        // An emergency stop bypasses this lock. Queued requests re-check pause after acquiring it.
        return synchronized(actionLock) { perform(body, action) }
    }

    private fun perform(body: JsonObject, action: String): JsonObject {
        if (action !in ACTIONS) bad("unsupported phone action: ${action.take(40)}")
        if (action == "middle_click") bad("middle_click is not available on a touch screen")
        if (action !in READ_ONLY && paused) throw BridgeError(423, "phone control is paused; resume it on the phone before sending input")
        val s = control.status()
        if (!s.helperConnected || s.displayId == null || s.width <= 0 || s.height <= 0) {
            throw BridgeError(503, "phone control is not connected; finish phone-control setup in the Cranibox app")
        }
        val screen = Screen(s.width, s.height, body.string("coordinate_space") ?: "pixels")
        return when (action) {
            "screenshot" -> image(control.screenshot(null))
            "zoom" -> {
                val r = screen.region(body["region"] ?: bad("region is required"))
                image(control.screenshot(r))
            }
            "cursor_position" -> {
                val p = point()
                buildJsonObject { put("result", "ok"); put("x", p[0]); put("y", p[1]) }
            }
            "mouse_move" -> {
                moveTo(screen.point(body["coordinate"] ?: bad("coordinate is required")))
                result()
            }
            "left_click" -> target(body, screen).let { p -> touched(control.tap(p[0], p[1])) }
            "double_click" -> {
                val p = target(body, screen)
                touched(control.tap(p[0], p[1]) && Thread.sleep(DOUBLE_TAP_GAP_MS).let { !paused && control.tap(p[0], p[1]) })
            }
            "right_click" -> {
                val p = target(body, screen)
                touched(control.stroke(Gestures.longPress(p[0], p[1])))
            }
            "left_click_drag" -> {
                val from = screen.point(body["start_coordinate"] ?: bad("start_coordinate is required"))
                val to = screen.point(body["coordinate"] ?: bad("coordinate is required"))
                moveTo(to)
                touched(control.stroke(Gestures.drag(from, to)))
            }
            "scroll" -> {
                val direction = body.string("scroll_direction") ?: bad("scroll_direction is required")
                val amount = body.int("scroll_amount") ?: bad("scroll_amount is required")
                if (amount !in 1..100) bad("scroll amount must be 1–100")
                val at = body["coordinate"]?.takeIf { it !is JsonNull }?.let(screen::point)
                touched(control.stroke(Gestures.scroll(direction, amount, at, s.width, s.height)))
            }
            "type" -> {
                val text = body.string("text") ?: bad("text is required")
                if (text.isEmpty() || text.toByteArray().size > 16 * 1024 || '\u0000' in text) bad("text must contain 1–16384 bytes without NUL")
                val keys = text.all { it == '\n' || it == '\t' || it in ' '..'~' }
                if (!(if (keys) control.typeAscii(text) else control.paste(text))) inputRejected()
                buildJsonObject { put("result", "ok"); put("method", if (keys) "keys" else "clipboard") }
            }
            "key" -> {
                val text = body.string("text") ?: bad("text is required")
                val chord = Keys.parse(text) ?: bad("key not allowed on the phone: ${text.take(64)}")
                touched(control.key(chord.keyCode, chord.metaState))
            }
            "window_list" -> buildJsonObject {
                put("result", "ok")
                putJsonArray("windows") {
                    control.tasks().forEach { t -> add(buildJsonObject { put("id", t.id.toString()); put("title", t.packageName); put("package", t.packageName) }) }
                }
                put("complete", true)
            }
            "window_focus" -> {
                val id = body.string("window_id")?.takeIf { it.length in 1..10 && it.all(Char::isDigit) }?.toIntOrNull()
                    ?: bad("window ID must come from window_list")
                touched(control.focusTask(id))
            }
            "app_list" -> buildJsonObject {
                put("result", "ok")
                putJsonArray("apps") {
                    control.apps().sortedBy { it.label.lowercase() }.forEach { a -> add(buildJsonObject { put("package", a.packageName); put("label", a.label) }) }
                }
            }
            "open_app" -> {
                val pkg = body.string("package") ?: bad("package is required")
                if (!validPackage(pkg)) bad("package must be an Android package name such as com.example.app")
                val r = control.openApp(pkg)
                // The Rust caller rejects bodies with an "error" key; report outcomes in "result".
                buildJsonObject { put("result", r.wire); put("ok", r == OpenAppResult.STARTED); put("package", pkg) }
            }
            else -> bad("unsupported phone action: ${action.take(40)}")
        }
    }

    private fun target(body: JsonObject, screen: Screen): IntArray =
        body["coordinate"]?.takeIf { it !is JsonNull }?.let(screen::point)?.also(::moveTo) ?: point()

    private fun touched(accepted: Boolean): JsonObject = if (accepted) result() else inputRejected()
    private fun inputRejected(): Nothing = throw BridgeError(503, "the phone did not accept the input")

    private fun point(): IntArray = synchronized(lock) {
        last?.copyOf() ?: control.status().let { intArrayOf(it.width / 2, it.height / 2) }
    }

    private fun moveTo(p: IntArray) = synchronized(lock) { last = p.copyOf() }

    private fun record(session: String, action: String) = synchronized(lock) {
        val id = session.ifBlank { "http-client" }.take(80)
        sessions.remove(id)
        sessions[id] = action.take(40) to clock()
        while (sessions.size > MAX_SESSIONS) sessions.remove(sessions.keys.first())
    }

    private fun image(png: Png?): JsonObject {
        png ?: throw BridgeError(503, "no frame from the phone display yet")
        if (png.bytes.size > (8 * 1024 * 1024 - 1024) * 3 / 4) throw BridgeError(503, "phone screenshot exceeds the 8 MiB response limit")
        return buildJsonObject {
            put("result", "ok")
            put("data", Base64.getEncoder().encodeToString(png.bytes))
            put("width", png.width)
            put("height", png.height)
            put("mime_type", "image/png")
        }
    }

    private fun result() = buildJsonObject { put("result", "ok") }

    private fun ok(p: PauseState) = buildJsonObject {
        put("result", "ok"); put("paused", p.paused); put("reason", p.reason); put("changed_at", p.changedAt)
    }

    /** Coordinates in pixels or normalized to [0, 1], checked against the display. */
    private class Screen(val width: Int, val height: Int, val space: String) {
        init {
            if (space != "pixels" && space != "normalized") bad("coordinate_space must be pixels or normalized")
        }

        fun point(value: JsonElement): IntArray {
            val v = numbers(value, 2, "coordinate")
            val p = if (space == "normalized") {
                if (v.any { it < 0 || it > 1 }) bad("coordinate must contain two normalized values between 0 and 1")
                intArrayOf((v[0] * (width - 1)).roundToInt(), (v[1] * (height - 1)).roundToInt())
            } else {
                intArrayOf(v[0].roundToInt(), v[1].roundToInt())
            }
            if (p[0] !in 0 until width || p[1] !in 0 until height) bad("coordinate is outside the phone display")
            return p
        }

        fun region(value: JsonElement): IntArray {
            val v = numbers(value, 4, "region")
            val r = if (space == "normalized") {
                if (v.any { it < 0 || it > 1 }) bad("region must contain four normalized values between 0 and 1")
                intArrayOf((v[0] * width).roundToInt(), (v[1] * height).roundToInt(), (v[2] * width).roundToInt(), (v[3] * height).roundToInt())
            } else {
                IntArray(4) { v[it].roundToInt() }
            }
            if (r[0] < 0 || r[1] < 0 || r[2] > width || r[3] > height || r[0] >= r[2] || r[1] >= r[3]) bad("invalid screenshot region")
            return r
        }

        private fun numbers(value: JsonElement, n: Int, name: String): DoubleArray {
            val list = (value as? JsonArray)?.takeIf { it.size == n } ?: bad("$name must contain $n numbers")
            return DoubleArray(n) { i ->
                (list[i] as? JsonPrimitive)?.takeIf { !it.isString }?.doubleOrNull?.takeIf { it.isFinite() } ?: bad("$name must contain $n numbers")
            }
        }
    }

    companion object {
        const val SESSION_TTL_MS = 300_000L
        private const val MAX_SESSIONS = 32
        private const val DOUBLE_TAP_GAP_MS = 80L
        private val READ_ONLY = setOf("screenshot", "zoom", "cursor_position", "window_list", "app_list")
        private val ACTIONS = READ_ONLY + setOf(
            "mouse_move", "left_click", "right_click", "middle_click", "double_click", "left_click_drag",
            "scroll", "type", "key", "window_focus", "open_app",
        )
        val CAPABILITIES = listOf(
            "screenshot", "zoom", "cursor_position", "click", "double_click", "long_press", "drag", "scroll",
            "key", "type", "window_list", "window_focus", "app_list", "open_app", "emergency_stop",
        )

        /** Same rule as the node's `valid_package`: dot-separated identifiers, at least two segments. */
        fun validPackage(name: String): Boolean {
            val parts = name.split('.')
            return name.length <= 255 && parts.size >= 2 && parts.all { p ->
                p.isNotEmpty() && p[0].isLetter() && p[0].code < 128 && p.all { it == '_' || (it.code < 128 && it.isLetterOrDigit()) }
            }
        }
    }
}

private fun JsonObject.string(key: String): String? = (this[key] as? JsonPrimitive)?.takeIf { it.isString }?.content

private fun JsonObject.int(key: String): Int? =
    (this[key] as? JsonPrimitive)?.takeIf { !it.isString }?.doubleOrNull?.takeIf { it == Math.floor(it) && it in -1e9..1e9 }?.toInt()
