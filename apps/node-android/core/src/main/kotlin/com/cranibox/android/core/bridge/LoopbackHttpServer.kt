package com.cranibox.android.core.bridge

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import java.io.BufferedInputStream
import java.io.ByteArrayOutputStream
import java.io.Closeable
import java.io.IOException
import java.io.InputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketException
import java.security.MessageDigest
import java.util.concurrent.ExecutorService
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.ThreadPoolExecutor
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/**
 * A small HTTP/1.1 server for the bridge, bound to 127.0.0.1 only. Every app on the phone can reach
 * loopback, so each request must carry `Authorization: Bearer <token>`; browsers (any Origin header) are refused.
 * One request per connection.
 */
class LoopbackHttpServer(
    private val requestedPort: Int,
    private val token: String,
    private val bridge: Bridge,
) : Closeable {
    private val expected = "Bearer $token".toByteArray(Charsets.UTF_8)
    private var socket: ServerSocket? = null
    private var pool: ExecutorService? = null
    private val clients = ConcurrentHashMap.newKeySet<Socket>()

    init {
        require(token.length >= 16) { "bridge token is too short" }
    }

    val port: Int get() = socket?.localPort ?: error("server not started")

    @Synchronized
    fun start() {
        check(socket == null) { "already started" }
        val server = ServerSocket(requestedPort, 16, InetAddress.getByName("127.0.0.1"))
        val workers = ThreadPoolExecutor(4, 4, 0, TimeUnit.MILLISECONDS, ArrayBlockingQueue(16),
            { r -> Thread(r, "cranibox-bridge").apply { isDaemon = true } }, ThreadPoolExecutor.AbortPolicy())
        socket = server
        pool = workers
        thread(name = "cranibox-bridge-accept", isDaemon = true) {
            while (!server.isClosed) {
                val client = try {
                    server.accept()
                } catch (_: IOException) {
                    break
                }
                clients.add(client)
                try {
                    workers.execute { try { client.use(::serve) } finally { clients.remove(client) } }
                } catch (_: Exception) {
                    clients.remove(client)
                    client.close()
                }
            }
        }
    }

    @Synchronized
    override fun close() {
        runCatching { socket?.close() }
        clients.forEach { runCatching { it.close() } }
        clients.clear()
        pool?.shutdownNow()
        pool?.awaitTermination(2, TimeUnit.SECONDS)
        socket = null
        pool = null
    }

    private fun serve(client: Socket) {
        client.soTimeout = READ_TIMEOUT_MS
        val out = client.getOutputStream()
        val (status, body) = try {
            respond(BufferedInputStream(client.getInputStream()))
        } catch (_: SocketException) {
            return
        } catch (e: HttpError) {
            e.status to errorBody(e.message)
        } catch (_: IOException) {
            400 to errorBody("malformed request")
        }
        var responseStatus = status
        var bytes = body.toString().toByteArray(Charsets.UTF_8)
        if (bytes.size > 8 * 1024 * 1024) {
            responseStatus = 503
            bytes = errorBody("response exceeds 8 MiB").toString().toByteArray(Charsets.UTF_8)
        }
        val head = "HTTP/1.1 $responseStatus ${reason(responseStatus)}\r\n" +
            "Content-Type: application/json; charset=utf-8\r\n" +
            "Content-Length: ${bytes.size}\r\n" +
            "Cache-Control: no-store\r\n" +
            "Connection: close\r\n\r\n"
        runCatching {
            out.write(head.toByteArray(Charsets.US_ASCII))
            out.write(bytes)
            out.flush()
        }
    }

    private fun respond(input: InputStream): Pair<Int, JsonObject> {
        val head = readHead(input)
        val lines = head.split("\r\n")
        val parts = lines.first().split(' ')
        if (parts.size != 3 || !parts[2].startsWith("HTTP/1.")) throw HttpError(400, "malformed request")
        val (method, path) = parts[0] to parts[1]
        val headers = mutableMapOf<String, String>()
        lines.drop(1).filter(String::isNotEmpty).forEach { line ->
            val i = line.indexOf(':')
            if (i <= 0) throw HttpError(400, "malformed header")
            val name = line.substring(0, i).trim().lowercase()
            if (headers.put(name, line.substring(i + 1).trim()) != null) throw HttpError(400, "duplicate header")
        }
        if ("origin" in headers) throw HttpError(403, "browser requests are not accepted")
        val auth = (headers["authorization"] ?: "").toByteArray(Charsets.UTF_8)
        if (!MessageDigest.isEqual(auth, expected)) throw HttpError(401, "unauthorized")
        return when {
            method == "GET" && (path == "/v1/status" || path == "/health") -> {
                val status = bridge.status()
                val healthy = status["backend_available"]?.toString() == "true"
                (if (path == "/health" && !healthy) 503 else 200) to status
            }
            method == "POST" && path == "/v1/action" -> {
                if (headers["transfer-encoding"] != null) throw HttpError(400, "chunked bodies are not accepted")
                val length = headers["content-length"]?.toIntOrNull() ?: throw HttpError(400, "Content-Length is required")
                if (length <= 0 || length > MAX_BODY) throw HttpError(400, "request body must be between 1 byte and 2 MiB")
                val body = ByteArray(length)
                var read = 0
                while (read < length) {
                    val n = input.read(body, read, length - read)
                    if (n < 0) throw HttpError(400, "request body is incomplete")
                    read += n
                }
                val json = try {
                    Json.parseToJsonElement(body.toString(Charsets.UTF_8)) as? JsonObject
                } catch (_: Exception) {
                    null
                } ?: throw HttpError(400, "JSON object required")
                val response = bridge.execute(json, headers["x-agent-session"] ?: "http-client")
                response.status to response.body
            }
            else -> throw HttpError(404, "not found")
        }
    }

    /** Reads up to the blank line ending the headers, at most [MAX_HEAD] bytes. */
    private fun readHead(input: InputStream): String {
        val buffer = ByteArrayOutputStream()
        var last4 = 0
        while (buffer.size() <= MAX_HEAD) {
            val b = input.read()
            if (b < 0) throw HttpError(400, "malformed request")
            buffer.write(b)
            last4 = (last4 shl 8) or b
            if (last4 == CRLFCRLF) return buffer.toByteArray().let { String(it, 0, it.size - 4, Charsets.ISO_8859_1) }
        }
        throw HttpError(400, "headers are too large")
    }

    private class HttpError(val status: Int, override val message: String) : IOException(message)

    private fun errorBody(message: String) = buildJsonObject { put("error", message) }

    private fun reason(status: Int) = when (status) {
        200 -> "OK"; 400 -> "Bad Request"; 401 -> "Unauthorized"; 403 -> "Forbidden"; 404 -> "Not Found"
        423 -> "Locked"; 503 -> "Service Unavailable"; else -> "Error"
    }

    companion object {
        const val MAX_BODY = 2 * 1024 * 1024
        private const val MAX_HEAD = 16 * 1024
        private const val READ_TIMEOUT_MS = 15_000
        private const val CRLFCRLF = 0x0d0a0d0a
    }
}
