package com.cranibox.android.ui.chat

import android.app.Application
import android.util.Log
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.cranibox.android.NodeApp
import com.cranibox.android.pico.ChatMessage
import com.cranibox.android.pico.ChatReducer
import com.cranibox.android.pico.ChatSession
import com.cranibox.android.pico.ChatUiState
import com.cranibox.android.pico.PicoClient
import com.cranibox.android.pico.PicoClawConfig
import com.cranibox.android.pico.PicoEvent
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.io.File
import java.util.UUID

class ChatViewModel(application: Application) : AndroidViewModel(application) {
    private val app = application as NodeApp
    private val client = PicoClient(tokenProvider = { app.pico.token })
    private val _state = MutableStateFlow(
        ChatUiState(sessionId = UUID.randomUUID().toString(), sessions = app.pico.loadSessions()),
    )
    val state = _state.asStateFlow()

    init {
        viewModelScope.launch {
            app.pico.ready.collectLatest { ready ->
                val settings = app.pico.loadSettings()
                _state.update {
                    it.copy(
                        runtimeReady = ready,
                        modelConfigured = settings.hasAnyModel,
                        modelLabel = settings.defaultModelName,
                        status = app.pico.status.value,
                    )
                }
                if (ready) connect(_state.value.sessionId) else client.close()
            }
        }
        viewModelScope.launch {
            app.pico.status.collectLatest { status -> _state.update { it.copy(status = status) } }
        }
        viewModelScope.launch {
            client.connected.collectLatest { connected ->
                _state.update { it.copy(connected = connected, connecting = false) }
            }
        }
        viewModelScope.launch {
            for (event in client.events) {
                if (event is PicoEvent.Assistant) logStreamFrame(event)
                _state.update { ChatReducer.apply(it, event) }
                if (event is PicoEvent.Assistant && event.kind != "thought") persistTitle()
            }
        }
        viewModelScope.launch {
            app.repository.approvals.collectLatest { list ->
                val now = System.currentTimeMillis() / 1000
                _state.update { it.copy(pendingApprovals = list.count { item -> item.expiresAt == null || item.expiresAt > now }) }
            }
        }
    }

    private val wsLog = File(app.filesDir, "picoclaw-ws.log")

    private fun logStreamFrame(event: PicoEvent.Assistant) {
        val line = "ws ${event.type} id=${event.messageId} kind=${event.kind} chars=${event.content.length} replace=${event.replace}"
        Log.i(TAG, line)
        runCatching {
            if (wsLog.length() > 256 * 1024) wsLog.writeText("")
            wsLog.appendText(line + "\n")
        }
    }

    private fun connect(sessionId: String) {

        _state.update { it.copy(connecting = true, sessionId = sessionId, error = null) }
        runCatching { client.connect(sessionId) }
            .onFailure { err -> _state.update { it.copy(connecting = false, error = err.message) } }
    }

    fun draft(value: String) { _state.update { it.copy(drafting = value) } }

    fun send(text: String = _state.value.drafting) {
        val content = text.trim()
        if (content.isEmpty()) return
        val state = _state.value
        if (!state.runtimeReady) {
            _state.update { it.copy(error = "请先在「本机」页启动节点") }
            return
        }
        if (!state.modelConfigured) {
            _state.update { it.copy(error = "请先配置模型") }
            return
        }
        val id = runCatching { client.send(content) }.getOrElse { err ->
            _state.update { it.copy(error = err.message) }
            return
        }
        val user = ChatMessage(id = id, role = ChatMessage.Role.User, content = content)
        _state.update {
            it.copy(
                messages = it.messages + user,
                drafting = "",
                typing = true,
                error = null,
                sessions = upsert(it.sessions, it.sessionId, content),
            )
        }
        persistTitle()
    }

    fun newSession() {
        val id = UUID.randomUUID().toString()
        persistTitle()
        _state.update { it.copy(sessionId = id, messages = emptyList(), typing = false, error = null, drafting = "") }
        if (_state.value.runtimeReady) connect(id)
    }

    fun openSession(id: String) {
        persistTitle()
        _state.update { it.copy(sessionId = id, messages = emptyList(), typing = false, error = null) }
        if (_state.value.runtimeReady) connect(id)
    }

    fun stopTyping() {
        // PicoClaw v0.3.1's Pico channel has no abort frame; hide the in-progress state locally.
        _state.update { it.copy(typing = false) }
    }

    fun saveModel(settings: PicoClawConfig.ModelSettings, onDone: (String?) -> Unit) {
        viewModelScope.launch {
            runCatching {
                app.pico.saveSettings(settings)
                app.pico.stop()
                app.work.launch { app.pico.start() }
            }.onSuccess { onDone(null) }.onFailure { onDone(it.message) }
        }
    }

    fun currentSettings() = app.pico.loadSettings()

    private fun upsert(sessions: List<ChatSession>, id: String, content: String): List<ChatSession> {
        val title = sessions.find { it.id == id }?.title?.takeIf { it.isNotBlank() && it != "新对话" }
            ?: content.take(24)
        val next = ChatSession(id, title, System.currentTimeMillis())
        return listOf(next) + sessions.filterNot { it.id == id }
    }

    private fun persistTitle() {
        val state = _state.value
        val title = state.messages.firstOrNull { it.role == ChatMessage.Role.User }?.content?.take(24) ?: "新对话"
        val sessions = upsert(state.sessions, state.sessionId, title)
        _state.update { it.copy(sessions = sessions) }
        app.pico.saveSessions(sessions)
    }

    override fun onCleared() {
        client.close()
        super.onCleared()
    }

    private companion object {
        const val TAG = "CraniboxPico"
    }
}
