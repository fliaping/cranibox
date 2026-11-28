package com.cranibox.android.ui.chat

import com.cranibox.android.pico.ChatMessage
import com.cranibox.android.pico.ChatReducer
import com.cranibox.android.pico.ChatUiState
import com.cranibox.android.pico.PicoEvent
import com.cranibox.android.pico.PicoProtocol
import com.cranibox.android.pico.ToolCall
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class ChatReducerTest {
    private val start = ChatUiState(sessionId = "s")

    @Test fun streamsUpdatesAndIgnoresOtherSessions() {
        val created = ChatReducer.apply(
            start,
            PicoEvent.Assistant("message.create", "m1", "s", "你", PicoProtocol.KIND_NORMAL, null, emptyList(), false, 1, false),
        )
        val updated = ChatReducer.apply(
            created,
            PicoEvent.Assistant("message.update", "m1", "s", "你好", PicoProtocol.KIND_NORMAL, "custom", emptyList(), false, 2, true),
        )
        assertEquals("你好", updated.messages.single().content)
        assertEquals("custom", updated.messages.single().modelName)
        val ignored = ChatReducer.apply(
            updated,
            PicoEvent.Assistant("message.create", "other", "nope", "x", PicoProtocol.KIND_NORMAL, null, emptyList(), false, 3, false),
        )
        assertEquals(1, ignored.messages.size)
    }

    @Test fun recordsToolPillsAndClearsTypingOnFinal() {
        val typing = ChatReducer.apply(start, PicoEvent.Typing(true, "s"))
        assertTrue(typing.typing)
        val tools = ChatReducer.apply(
            typing,
            PicoEvent.Assistant("message.create", "t1", "s", "", PicoProtocol.KIND_TOOL_CALLS, null, listOf(ToolCall("cranibox_devices", "running")), false, 1, false),
        )
        assertEquals("cranibox_devices", tools.messages.single().toolCalls.single().name)
        val done = ChatReducer.apply(
            tools,
            PicoEvent.Assistant("message.create", "m2", "s", "两台设备在线", PicoProtocol.KIND_NORMAL, null, emptyList(), false, 2, false),
        )
        assertFalse(done.typing)
        assertEquals(2, done.messages.size)
    }

    @Test fun errorDropsRequestAndStopsTyping() {
        val withUser = start.copy(messages = listOf(ChatMessage("req", ChatMessage.Role.User, "hi")), typing = true)
        val erred = ChatReducer.apply(withUser, PicoEvent.Error("empty_content", "message content is empty", "req", "s"))
        assertTrue(erred.messages.isEmpty())
        assertFalse(erred.typing)
        assertEquals("message content is empty", erred.error)
    }
}
