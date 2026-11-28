package com.cranibox.android.pico

import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertIs
import kotlin.test.assertTrue

class PicoProtocolTest {
    @Test fun websocketUrlUsesVerifiedPath() {
        val url = PicoProtocol.websocketUrl(sessionId = "abc def")
        assertEquals("ws://127.0.0.1:18790/pico/ws?session_id=abc%20def", url)
        assertTrue(PicoProtocol.isLoopbackWs(url))
        assertFalse(PicoProtocol.isLoopbackWs("ws://example.com:18790/pico/ws"))
        assertFalse(PicoProtocol.isLoopbackWs("ws://127.0.0.1:18790/other"))
    }

    @Test fun encodesMessageSendMatchingV031() {
        val raw = PicoProtocol.encodeSend("req-1", "sess-1", "查看我的在线设备")
        val obj = PicoProtocol.json.parseToJsonElement(raw).jsonObject
        assertEquals("message.send", obj["type"]?.jsonPrimitive?.content)
        assertEquals("req-1", obj["id"]?.jsonPrimitive?.content)
        assertEquals("sess-1", obj["session_id"]?.jsonPrimitive?.content)
        assertEquals("查看我的在线设备", obj["payload"]!!.jsonObject["content"]?.jsonPrimitive?.content)
    }

    @Test fun parsesAssistantCreateUpdateAndTools() {
        val create = PicoProtocol.parse(
            """{"type":"message.create","session_id":"s","payload":{"message_id":"m1","content":"你好","model_name":"custom"}}""",
        )
        val created = assertIs<PicoEvent.Assistant>(create)
        assertEquals("m1", created.messageId)
        assertEquals("你好", created.content)
        assertEquals("custom", created.modelName)
        assertFalse(created.replace)

        val update = PicoProtocol.parse(
            """{"type":"message.update","session_id":"s","payload":{"message_id":"m1","content":"你好，世界","kind":"normal"}}""",
        )
        assertTrue(assertIs<PicoEvent.Assistant>(update).replace)

        val tools = PicoProtocol.parse(
            """{"type":"message.create","payload":{"message_id":"t1","kind":"tool_calls","content":"running","tool_calls":[{"name":"cranibox_devices","status":"running"}]}}""",
        )
        val call = assertIs<PicoEvent.Assistant>(tools)
        assertEquals(PicoProtocol.KIND_TOOL_CALLS, call.kind)
        assertEquals("cranibox_devices", call.toolCalls.single().name)
    }

    @Test fun parsesThoughtTypingErrorAndDelete() {
        val thought = assertIs<PicoEvent.Assistant>(
            PicoProtocol.parse("""{"type":"message.create","payload":{"message_id":"th","content":"思考","thought":true}}"""),
        )
        assertEquals(PicoProtocol.KIND_THOUGHT, thought.kind)
        assertTrue(assertIs<PicoEvent.Typing>(PicoProtocol.parse("""{"type":"typing.start"}""")).typing)
        assertFalse(assertIs<PicoEvent.Typing>(PicoProtocol.parse("""{"type":"typing.stop"}""")).typing)
        val err = assertIs<PicoEvent.Error>(PicoProtocol.parse("""{"type":"error","payload":{"code":"empty_content","message":"message content is empty","request_id":"req"}}"""))
        assertEquals("req", err.requestId)
        assertEquals("m1", assertIs<PicoEvent.Delete>(PicoProtocol.parse("""{"type":"message.delete","payload":{"message_id":"m1"}}""")).messageId)
        assertIs<PicoEvent.Pong>(PicoProtocol.parse("""{"type":"pong","id":"1"}"""))
    }
}
