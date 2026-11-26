package com.cranibox.android.pico

data class ChatMessage(
    val id: String,
    val role: Role,
    val content: String,
    val kind: String = PicoProtocol.KIND_NORMAL,
    val modelName: String? = null,
    val toolCalls: List<ToolCall> = emptyList(),
    val timestamp: Long = System.currentTimeMillis(),
    val error: Boolean = false,
) {
    enum class Role { User, Assistant }
}

data class ChatSession(val id: String, val title: String, val updatedAt: Long)

data class ChatUiState(
    val sessionId: String,
    val sessions: List<ChatSession> = emptyList(),
    val messages: List<ChatMessage> = emptyList(),
    val typing: Boolean = false,
    val connected: Boolean = false,
    val connecting: Boolean = false,
    val runtimeReady: Boolean = false,
    val modelConfigured: Boolean = false,
    val modelLabel: String = "",
    val status: String = "",
    val error: String? = null,
    val pendingApprovals: Int = 0,
    val drafting: String = "",
) {
    val empty: Boolean get() = messages.isEmpty() && !typing
}

object ChatReducer {
    fun apply(state: ChatUiState, event: PicoEvent): ChatUiState {
        if (event.sessionId != null && event.sessionId != state.sessionId) return state
        return when (event) {
            is PicoEvent.Assistant -> {
                val existing = state.messages.indexOfFirst { it.id == event.messageId }
                val message = ChatMessage(
                    id = event.messageId,
                    role = ChatMessage.Role.Assistant,
                    content = event.content,
                    kind = event.kind,
                    modelName = event.modelName,
                    toolCalls = event.toolCalls,
                    timestamp = event.timestamp ?: System.currentTimeMillis(),
                )
                val messages = if (existing >= 0) {
                    state.messages.toMutableList().also { it[existing] = message.merge(state.messages[existing], event.replace) }
                } else state.messages + message
                state.copy(
                    messages = messages,
                    typing = if (!event.placeholder && event.kind == PicoProtocol.KIND_NORMAL) false else state.typing,
                    error = null,
                )
            }
            is PicoEvent.Delete -> state.copy(messages = state.messages.filterNot { it.id == event.messageId })
            is PicoEvent.Typing -> state.copy(typing = event.typing)
            is PicoEvent.Error -> {
                val without = event.requestId?.let { id -> state.messages.filterNot { it.id == id } } ?: state.messages
                state.copy(messages = without, typing = false, error = event.message.ifBlank { "Agent 出错了" })
            }
            is PicoEvent.Pong, is PicoEvent.Unknown -> state
        }
    }

    private fun ChatMessage.merge(previous: ChatMessage, replace: Boolean): ChatMessage {
        val content = when {
            !replace && this.content.isBlank() -> previous.content
            else -> this.content
        }
        val tools = toolCalls.ifEmpty { previous.toolCalls }
        val kind = if (kind == PicoProtocol.KIND_NORMAL && previous.kind != PicoProtocol.KIND_NORMAL && content == previous.content) previous.kind else kind
        return copy(content = content, toolCalls = tools, kind = kind, modelName = modelName ?: previous.modelName)
    }
}
