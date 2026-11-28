package com.cranibox.android.core.bridge

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.Base64

class FakePhone : PhoneControl {
    var connected = true
    var accept = true
    val calls = mutableListOf<String>()
    val png = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte())
    override fun status() = PhoneStatus(connected, if (connected) 7 else null, 1080, 2400, true)
    override fun screenshot(region: IntArray?): Png? {
        calls += "screenshot ${region?.joinToString(",")}"
        return Png(png, region?.let { it[2] - it[0] } ?: 1080, region?.let { it[3] - it[1] } ?: 2400)
    }
    override fun tap(x: Int, y: Int) = accept.also { calls += "tap $x $y" }
    override fun stroke(points: List<StrokePoint>) = accept.also { calls += "stroke ${points.first().x},${points.first().y}->${points.last().x},${points.last().y}" }
    override fun key(keyCode: Int, metaState: Int) = accept.also { calls += "key $keyCode $metaState" }
    override fun typeAscii(text: String) = accept.also { calls += "keys $text" }
    override fun paste(text: String) = accept.also { calls += "paste $text" }
    override fun tasks() = listOf(PhoneTask(42, "com.tencent.mm"))
    override fun focusTask(id: Int) = accept.also { calls += "focus $id" }
    override fun apps() = listOf(PhoneApp("com.b", "beta"), PhoneApp("com.a", "Alpha"))
    override fun openApp(packageName: String) = OpenAppResult.STARTED.also { calls += "open $packageName" }
}

class MemoryPause : PauseStore {
    var state = PauseState(false)
    override fun load() = state
    override fun save(state: PauseState) { this.state = state }
}

private fun obj(s: String) = Json.parseToJsonElement(s).jsonObject
private fun JsonObject.str(k: String) = this[k]!!.jsonPrimitive.content

class BridgeTest {
    private val phone = FakePhone()
    private val pause = MemoryPause()
    private var now = 1_000_000L
    private val bridge = Bridge(phone, pause) { now }
    private fun run(s: String, session: String = "agent") = bridge.execute(obj(s), session)

    @Test fun statusReportsReadinessAndSessions() {
        val r = run("""{"action":"status"}""", "codex-1")
        assertEquals(200, r.status)
        assertEquals("ready", r.body.str("state"))
        assertEquals("android", r.body.str("platform"))
        assertEquals("1080", r.body["resolution"]!!.jsonObject.str("width"))
        assertEquals("7", r.body.str("display_id"))
        assertEquals("codex-1", r.body["active_sessions"]!!.jsonArray[0].jsonObject.str("id"))
        now += Bridge.SESSION_TTL_MS + 1
        assertTrue(bridge.status()["active_sessions"]!!.jsonArray.isEmpty())
        phone.connected = false
        assertEquals("backend-unavailable", bridge.status().str("state"))
    }

    @Test fun pauseBlocksInputButNotScreenshots() {
        assertEquals(200, run("""{"action":"emergency_stop"}""").status)
        assertTrue(pause.state.paused)
        assertEquals("emergency stop", pause.state.reason)
        val blocked = run("""{"action":"left_click","coordinate":[10,10]}""")
        assertEquals(423, blocked.status)
        assertEquals("true", blocked.body.str("paused"))
        assertEquals(200, run("""{"action":"screenshot"}""").status)
        assertEquals(423, run("""{"action":"open_app","package":"com.a.b"}""").status)
        assertEquals("paused", run("""{"action":"status"}""").body.str("state"))
        assertEquals(200, run("""{"action":"resume"}""").status)
        assertFalse(pause.state.paused)
        assertEquals(200, run("""{"action":"left_click","coordinate":[10,10]}""").status)
        assertEquals("tap 10 10", phone.calls.last())
    }

    @Test fun pauseSurvivesRestart() {
        bridge.emergencyStop("owner")
        assertTrue(Bridge(phone, pause).paused)
    }

    @Test fun emergencyStopBypassesActiveInputAndRejectsQueuedInput() {
        val entered = java.util.concurrent.CountDownLatch(1)
        val release = java.util.concurrent.CountDownLatch(1)
        val calls = java.util.concurrent.atomic.AtomicInteger()
        val stopped = java.util.concurrent.atomic.AtomicBoolean()
        val blockingPhone = object : PhoneControl by phone {
            override fun setPaused(paused: Boolean) { stopped.set(paused) }
            override fun tap(x: Int, y: Int): Boolean {
                calls.incrementAndGet()
                entered.countDown()
                check(release.await(5, java.util.concurrent.TimeUnit.SECONDS))
                return true
            }
        }
        val b = Bridge(blockingPhone, pause)
        val pool = java.util.concurrent.Executors.newFixedThreadPool(2)
        try {
            val first = pool.submit<BridgeResponse> { b.execute(obj("""{"action":"left_click","coordinate":[1,1]}"""), "first") }
            assertTrue(entered.await(2, java.util.concurrent.TimeUnit.SECONDS))
            val queued = pool.submit<BridgeResponse> { b.execute(obj("""{"action":"left_click","coordinate":[2,2]}"""), "queued") }
            b.emergencyStop("owner")
            assertTrue(stopped.get())
            assertTrue(b.paused)
            release.countDown()
            assertEquals(200, first.get(2, java.util.concurrent.TimeUnit.SECONDS).status)
            assertEquals(423, queued.get(2, java.util.concurrent.TimeUnit.SECONDS).status)
            assertEquals(1, calls.get())
        } finally { release.countDown(); pool.shutdownNow() }
    }

    @Test fun stopDuringDoubleTapSuppressesTheSecondTap() {
        var taps = 0
        lateinit var b: Bridge
        val stoppingPhone = object : PhoneControl by phone {
            override fun tap(x: Int, y: Int): Boolean { taps++; b.emergencyStop("owner"); return true }
        }
        b = Bridge(stoppingPhone, pause)
        b.execute(obj("""{"action":"double_click","coordinate":[1,1]}"""), "agent")
        assertEquals(1, taps)
    }

    @Test fun disconnectedHelperIsUnavailable() {
        phone.connected = false
        assertEquals(503, run("""{"action":"screenshot"}""").status)
        assertEquals(200, run("""{"action":"status"}""").status)
        assertEquals(200, run("""{"action":"emergency_stop"}""").status)
    }

    @Test fun normalizedCoordinatesAndBounds() {
        assertEquals(200, run("""{"action":"left_click","coordinate":[0.5,1],"coordinate_space":"normalized"}""").status)
        assertEquals("tap 540 2399", phone.calls.last())
        assertEquals(400, run("""{"action":"left_click","coordinate":[1.5,0],"coordinate_space":"normalized"}""").status)
        assertEquals(400, run("""{"action":"left_click","coordinate":[1080,0]}""").status)
        assertEquals(400, run("""{"action":"left_click","coordinate":["1",0]}""").status)
        assertEquals(400, run("""{"action":"left_click","coordinate":[1,2],"coordinate_space":"inches"}""").status)
        // A click without a coordinate uses the last point.
        assertEquals(200, run("""{"action":"left_click"}""").status)
        assertEquals("tap 540 2399", phone.calls.last())
        val cursor = run("""{"action":"cursor_position"}""").body
        assertEquals("540", cursor.str("x"))
    }

    @Test fun gesturesAndKeys() {
        assertEquals(200, run("""{"action":"double_click","coordinate":[5,6]}""").status)
        assertEquals(listOf("tap 5 6", "tap 5 6"), phone.calls.takeLast(2))
        run("""{"action":"right_click","coordinate":[5,6]}""")
        assertEquals("stroke 5,6->5,6", phone.calls.last())
        run("""{"action":"left_click_drag","start_coordinate":[1,2],"coordinate":[3,4]}""")
        assertEquals("stroke 1,2->3,4", phone.calls.last())
        assertEquals(200, run("""{"action":"scroll","scroll_direction":"down","scroll_amount":3}""").status)
        assertEquals(400, run("""{"action":"scroll","scroll_direction":"down","scroll_amount":0}""").status)
        assertEquals(400, run("""{"action":"scroll","scroll_direction":"sideways","scroll_amount":3}""").status)
        assertEquals(200, run("""{"action":"key","text":"Return"}""").status)
        assertEquals("key 66 0", phone.calls.last())
        val home = run("""{"action":"key","text":"super"}""")
        assertEquals(400, home.status)
        assertEquals(400, run("""{"action":"middle_click"}""").status)
        assertEquals(400, run("""{"action":"hold_key","text":"a"}""").status)
        assertEquals(400, run("""{"action":"zoom","region":[10,10,5,20]}""").status)
    }

    @Test fun typingRoutesNonAsciiThroughTheClipboard() {
        assertEquals("keys", run("""{"action":"type","text":"hello world\n"}""").body.str("method"))
        assertEquals("clipboard", run("""{"action":"type","text":"你好"}""").body.str("method"))
        assertEquals("paste 你好", phone.calls.last())
        assertEquals(400, run("""{"action":"type","text":""}""").status)
    }

    @Test fun refusedInputIsReported() {
        phone.accept = false
        val r = run("""{"action":"left_click","coordinate":[1,1]}""")
        assertEquals(503, r.status)
        assertTrue(r.body.containsKey("error"))
    }

    @Test fun screenshotIsBase64Png() {
        val r = run("""{"action":"screenshot"}""").body
        assertEquals("image/png", r.str("mime_type"))
        assertEquals(phone.png.toList(), Base64.getDecoder().decode(r.str("data")).toList())
        val z = run("""{"action":"zoom","region":[0.0,0.0,0.5,0.5],"coordinate_space":"normalized"}""").body
        assertEquals("540", z.str("width"))
        assertEquals("screenshot 0,0,540,1200", phone.calls.last())
    }

    @Test fun appsAndWindows() {
        val apps = run("""{"action":"app_list"}""").body["apps"]!!.jsonArray
        assertEquals("com.a", apps[0].jsonObject.str("package"))
        val opened = run("""{"action":"open_app","package":"com.tencent.mm"}""").body
        assertEquals("started", opened.str("result"))
        assertFalse(opened.containsKey("error"))
        assertEquals(400, run("""{"action":"open_app","package":"mm"}""").status)
        assertEquals(400, run("""{"action":"open_app","package":"com.x;reboot"}""").status)
        assertEquals("42", run("""{"action":"window_list"}""").body["windows"]!!.jsonArray[0].jsonObject.str("id"))
        assertEquals(200, run("""{"action":"window_focus","window_id":"42"}""").status)
        assertEquals(400, run("""{"action":"window_focus","window_id":"x"}""").status)
    }
}

class InputTest {
    @Test fun keyAllowlist() {
        assertEquals(KeyChord(67, 0), Keys.parse("BackSpace"))
        assertEquals(KeyChord(93, 0), Keys.parse("Page_Down"))
        assertEquals(KeyChord(29, 0x3000), Keys.parse("ctrl+a"))
        assertEquals(KeyChord(50, 0x3000), Keys.parse("Control+V"))
        for (bad in listOf("super", "power", "volume_up", "app_switch", "ctrl+alt+del", "shift+a", "", "a".repeat(200), "ctrl+")) {
            assertNull(bad, Keys.parse(bad))
        }
    }

    @Test fun scrollMovesTheFingerAgainstTheContent() {
        val down = Gestures.scroll("down", 3, null, 1000, 2000)
        assertEquals(500, down.first().x)
        assertEquals(1000, down.first().y)
        assertTrue(down.last().y < down.first().y)
        assertEquals(1000 - 1000 * 3 / 12, down.last().y)
        assertTrue(Gestures.scroll("up", 3, null, 1000, 2000).last().y > 1000)
        assertTrue(Gestures.scroll("right", 3, null, 1000, 2000).last().x < 500)
        // Clamped to the display, and time never goes backwards.
        val edge = Gestures.scroll("down", 100, intArrayOf(10, 10), 1000, 2000)
        assertEquals(0, edge.last().y)
        assertTrue(edge.zipWithNext().all { (a, b) -> b.tMs >= a.tMs })
        assertEquals(0L, edge.first().tMs)
    }
}

class PauseStoreTest {
    @get:Rule val dir = TemporaryFolder()

    @Test fun roundTrip() {
        val file = File(dir.root, "state/pause.json")
        val store = FilePauseStore(file)
        assertFalse(store.load().paused)
        store.save(PauseState(true, "owner", "2026-11-02T00:00:00Z"))
        assertEquals(PauseState(true, "owner", "2026-11-02T00:00:00Z"), FilePauseStore(file).load())
        file.writeText("{broken")
        assertTrue(store.load().paused)
        file.writeText("{}")
        assertTrue(store.load().paused)
        store.save(PauseState(false))
        assertFalse(store.load().paused)
    }
}

class LoopbackHttpServerTest {
    private val token = BridgeToken.generate()

    private fun request(port: Int, method: String, path: String, body: String? = null, auth: String? = "Bearer $token" ): Pair<Int, String> {
        val c = URL("http://127.0.0.1:$port$path").openConnection() as HttpURLConnection
        c.requestMethod = method
        auth?.let { c.setRequestProperty("Authorization", it) }
        if (body != null) {
            c.doOutput = true
            c.outputStream.use { it.write(body.toByteArray()) }
        }
        val code = c.responseCode
        val text = (if (code < 400) c.inputStream else c.errorStream).use { it.readBytes().decodeToString() }
        return code to text
    }

    @Test fun authenticatesAndRoutes() {
        val phone = FakePhone()
        LoopbackHttpServer(0, token, Bridge(phone, MemoryPause())).use { server ->
            server.start()
            val p = server.port
            assertEquals(401, request(p, "GET", "/v1/status", auth = null).first)
            assertEquals(401, request(p, "GET", "/v1/status", auth = "Bearer wrong-token-0123456789").first)
            assertEquals(200, request(p, "GET", "/v1/status").first)
            assertEquals(404, request(p, "GET", "/other").first)
            // HttpURLConnection drops Origin, so send this one by hand.
            java.net.Socket("127.0.0.1", p).use { s ->
                s.getOutputStream().write("GET /v1/status HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer $token\r\nOrigin: http://evil.example\r\n\r\n".toByteArray())
                assertTrue(s.getInputStream().bufferedReader().readLine().startsWith("HTTP/1.1 403"))
            }
            val (code, text) = request(p, "POST", "/v1/action", """{"action":"left_click","coordinate":[3,4]}""")
            assertEquals(200, code)
            assertTrue(text.contains("\"ok\""))
            assertEquals("tap 3 4", phone.calls.last())
            assertEquals(400, request(p, "POST", "/v1/action", "[1]").first)
            // Reject an oversized Content-Length before reading its body. Sending the full body
            // with HttpURLConnection races the server's early close and may yield a broken pipe.
            java.net.Socket("127.0.0.1", p).use { s ->
                s.getOutputStream().write(("POST /v1/action HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer $token\r\n" +
                    "Content-Length: ${LoopbackHttpServer.MAX_BODY + 1}\r\n\r\n").toByteArray())
                assertTrue(s.getInputStream().bufferedReader().readLine().startsWith("HTTP/1.1 400"))
            }
            phone.connected = false
            assertEquals(503, request(p, "GET", "/health").first)
        }
    }
}
