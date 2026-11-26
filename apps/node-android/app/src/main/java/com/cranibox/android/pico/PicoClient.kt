package com.cranibox.android.pico

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.UUID
import java.util.concurrent.TimeUnit

/** Loopback-only OkHttp client for `ws://127.0.0.1:18790/pico/ws`. */
class PicoClient(
    private val tokenProvider: () -> String,
    private val client: OkHttpClient = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.SECONDS)
        .writeTimeout(20, TimeUnit.SECONDS)
        .pingInterval(20, TimeUnit.SECONDS)
        .build(),
) {
    val connected = MutableStateFlow(false)
    val events = Channel<PicoEvent>(Channel.BUFFERED)
    private var socket: WebSocket? = null
    @Volatile var sessionId: String = UUID.randomUUID().toString()
        private set

    fun connect(sessionId: String) {
        close()
        this.sessionId = sessionId
        val url = PicoProtocol.websocketUrl(sessionId = sessionId)
        check(PicoProtocol.isLoopbackWs(url))
        val token = tokenProvider()
        val request = Request.Builder()
            .url(url)
            .header("Authorization", "Bearer $token")
            .build()
        socket = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                connected.value = true
            }
            override fun onMessage(webSocket: WebSocket, text: String) {
                runCatching { PicoProtocol.parse(text) }.getOrNull()?.let { events.trySend(it) }
            }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
            }
            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                connected.value = false
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                connected.value = false
                events.trySend(PicoEvent.Error(code = "connection", message = t.message ?: "连接失败", requestId = null, sessionId = sessionId))
            }
        })
    }

    fun send(content: String): String {
        val id = UUID.randomUUID().toString()
        val body = PicoProtocol.encodeSend(id, sessionId, content)
        check(socket?.send(body) == true) { "Agent 未连接" }
        return id
    }

    fun ping() {
        socket?.send(PicoProtocol.encodePing(UUID.randomUUID().toString()))
    }

    fun close() {
        socket?.close(1000, "bye")
        socket = null
        connected.value = false
    }
}
