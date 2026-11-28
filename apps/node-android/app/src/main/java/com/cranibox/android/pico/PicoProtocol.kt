package com.cranibox.android.pico

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.net.URI

/**
 * Wire format for PicoClaw's native Pico channel (v0.3.1).
 *
 * The WebSocket endpoint is `ws://127.0.0.1:<port>/pico/ws?session_id=…`.
 * Authenticate with `Authorization: Bearer <token>` (or the `token.<token>`
 * subprotocol). Inbound events and the `message.send` payload were verified
 * against `pkg/channels/pico/protocol.go` and `pico.go` in tag v0.3.1.
 */
object PicoProtocol {
    const val DEFAULT_HOST = "127.0.0.1"
    const val DEFAULT_PORT = 18790
    const val WS_PATH = "/pico/ws"
    const val TYPE_MESSAGE_SEND = "message.send"
    const val TYPE_PING = "ping"
    const val TYPE_MESSAGE_CREATE = "message.create"
    const val TYPE_MESSAGE_UPDATE = "message.update"
    const val TYPE_MESSAGE_DELETE = "message.delete"
    const val TYPE_MEDIA_CREATE = "media.create"
    const val TYPE_TYPING_START = "typing.start"
    const val TYPE_TYPING_STOP = "typing.stop"
    const val TYPE_ERROR = "error"
    const val TYPE_PONG = "pong"
    const val KIND_THOUGHT = "thought"
    const val KIND_TOOL_CALLS = "tool_calls"
    const val KIND_NORMAL = "normal"

    val json = Json { ignoreUnknownKeys = true; encodeDefaults = false }

    fun websocketUrl(host: String = DEFAULT_HOST, port: Int = DEFAULT_PORT, sessionId: String): String {
        require(host == "127.0.0.1" || host == "localhost") { "PicoClaw is loopback-only" }
        require(port in 1..65535)
        val encoded = java.net.URLEncoder.encode(sessionId, "UTF-8").replace("+", "%20")
        return "ws://$host:$port$WS_PATH?session_id=$encoded"
    }

    fun isLoopbackWs(url: String): Boolean = runCatching {
        val uri = URI(url)
        val host = uri.host
        val scheme = uri.scheme
        (scheme == "ws" || scheme == "http") &&
            (host == "127.0.0.1" || host == "localhost") &&
            uri.port in 1..65535 &&
            uri.path.startsWith("/pico")
    }.getOrDefault(false)

    fun encodeSend(id: String, sessionId: String, content: String): String {
        val message = PicoWire(
            type = TYPE_MESSAGE_SEND,
            id = id,
            sessionId = sessionId,
            payload = mapOf("content" to JsonPrimitive(content)),
        )
        return json.encodeToString(PicoWire.serializer(), message)
    }

    fun encodePing(id: String): String =
        json.encodeToString(PicoWire.serializer(), PicoWire(type = TYPE_PING, id = id))

    fun parse(raw: String): PicoEvent = parse(json.parseToJsonElement(raw))

    fun parse(element: JsonElement): PicoEvent {
        val obj = element.jsonObject
        val type = obj["type"]?.jsonPrimitive?.contentOrNull.orEmpty()
        val sessionId = obj["session_id"]?.jsonPrimitive?.contentOrNull
        val id = obj["id"]?.jsonPrimitive?.contentOrNull
        val payload = obj["payload"]?.jsonObject ?: JsonObject(emptyMap())
        val timestamp = obj["timestamp"]?.jsonPrimitive?.contentOrNull?.toLongOrNull()
        return when (type) {
            TYPE_MESSAGE_CREATE, TYPE_MEDIA_CREATE, TYPE_MESSAGE_UPDATE -> {
                val messageId = payload["message_id"]?.jsonPrimitive?.contentOrNull ?: id ?: "pico-$type"
                val content = payload["content"]?.jsonPrimitive?.contentOrNull.orEmpty()
                val kind = payload["kind"]?.jsonPrimitive?.contentOrNull
                    ?: if (payload["thought"]?.jsonPrimitive?.booleanOrNull == true) KIND_THOUGHT else KIND_NORMAL
                val model = payload["model_name"]?.jsonPrimitive?.contentOrNull
                val tools = payload["tool_calls"]?.jsonArray?.mapNotNull { item ->
                    val o = item.jsonObject
                    val name = o["name"]?.jsonPrimitive?.contentOrNull
                        ?: o["tool"]?.jsonPrimitive?.contentOrNull
                        ?: o["function"]?.jsonObject?.get("name")?.jsonPrimitive?.contentOrNull
                    name?.let { tool ->
                        ToolCall(
                            name = tool,
                            status = o["status"]?.jsonPrimitive?.contentOrNull,
                            detail = o["content"]?.jsonPrimitive?.contentOrNull
                                ?: o["arguments"]?.toString(),
                        )
                    }
                }.orEmpty()
                PicoEvent.Assistant(
                    type = type,
                    messageId = messageId,
                    sessionId = sessionId,
                    content = content,
                    kind = kind,
                    modelName = model,
                    toolCalls = tools,
                    placeholder = payload["placeholder"]?.jsonPrimitive?.booleanOrNull == true,
                    timestamp = timestamp,
                    replace = type == TYPE_MESSAGE_UPDATE,
                )
            }
            TYPE_MESSAGE_DELETE -> PicoEvent.Delete(
                messageId = payload["message_id"]?.jsonPrimitive?.contentOrNull.orEmpty(),
                sessionId = sessionId,
            )
            TYPE_TYPING_START -> PicoEvent.Typing(true, sessionId)
            TYPE_TYPING_STOP -> PicoEvent.Typing(false, sessionId)
            TYPE_ERROR -> PicoEvent.Error(
                code = payload["code"]?.jsonPrimitive?.contentOrNull,
                message = payload["message"]?.jsonPrimitive?.contentOrNull.orEmpty(),
                requestId = payload["request_id"]?.jsonPrimitive?.contentOrNull,
                sessionId = sessionId,
            )
            TYPE_PONG -> PicoEvent.Pong(id)
            else -> PicoEvent.Unknown(type, sessionId)
        }
    }
}

@Serializable
data class PicoWire(
    val type: String,
    val id: String? = null,
    @SerialName("session_id") val sessionId: String? = null,
    val timestamp: Long? = null,
    val payload: Map<String, JsonElement>? = null,
)

data class ToolCall(val name: String, val status: String? = null, val detail: String? = null)

sealed class PicoEvent {
    abstract val sessionId: String?

    data class Assistant(
        val type: String,
        val messageId: String,
        override val sessionId: String?,
        val content: String,
        val kind: String,
        val modelName: String?,
        val toolCalls: List<ToolCall>,
        val placeholder: Boolean,
        val timestamp: Long?,
        val replace: Boolean,
    ) : PicoEvent()

    data class Delete(val messageId: String, override val sessionId: String?) : PicoEvent()
    data class Typing(val typing: Boolean, override val sessionId: String?) : PicoEvent()
    data class Error(val code: String?, val message: String, val requestId: String?, override val sessionId: String?) : PicoEvent()
    data class Pong(val id: String?) : PicoEvent() {
        override val sessionId: String? get() = null
    }
    data class Unknown(val type: String, override val sessionId: String?) : PicoEvent()
}
