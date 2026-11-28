package com.cranibox.android.api

import kotlinx.serialization.ExperimentalSerializationApi

import android.net.Uri
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.withContext
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import okhttp3.Cookie
import okhttp3.CookieJar
import okhttp3.HttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.TimeUnit

/** Loopback-only client for the node App API at 127.0.0.1. */
@OptIn(ExperimentalSerializationApi::class)
class NodeClient {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }
    private val jar = MemoryCookieJar()
    private val http = OkHttpClient.Builder()
        .cookieJar(jar)
        .followRedirects(true)
        .followSslRedirects(false)
        .connectTimeout(8, TimeUnit.SECONDS)
        .readTimeout(20, TimeUnit.SECONDS)
        .build()

    private val _ready = MutableStateFlow(false)
    val ready: StateFlow<Boolean> = _ready
    @Volatile var base: HttpUrl? = null
        private set
    @Volatile private var csrf: String = ""

    suspend fun disconnect() = withContext(Dispatchers.IO) {
        jar.clear()
        base = null
        csrf = ""
        _ready.value = false
    }

    suspend fun signIn(loginUrl: String) = withContext(Dispatchers.IO) {
        val url = loginUrl.takeIf { NodeUrl.isLocal(it) }
            ?: throw ApiException("登录链接不是本机地址")
        jar.clear()
        csrf = ""
        _ready.value = false
        val first = request(Request.Builder().url(url).header("Accept-Language", "zh-CN,zh;q=0.9").get().build(), signed = false)
        val origin = first.request.url.newBuilder().encodedPath("/").query(null).fragment(null).build()
        if (!isLoopback(origin)) throw ApiException("拒绝非本机节点地址")
        var html = first.body?.string().orEmpty()
        first.close()
        var token = Csrf.parse(html)
        if (token == null) {
            val page = request(Request.Builder().url(origin).header("Accept-Language", "zh-CN,zh;q=0.9").get().build(), signed = false)
            html = page.body?.string().orEmpty()
            page.close()
            token = Csrf.parse(html)
        }
        csrf = token ?: throw ApiException("无法读取节点登录凭证")
        base = origin
        _ready.value = true
    }

    suspend fun devices(): List<Device> = get("/api/v1/app/devices")
    suspend fun approvals(): List<ApprovalCard> = get("/api/v1/app/approvals")
    suspend fun tasks(): List<TaskSummary> = get("/api/v1/app/tasks")
    suspend fun access(): AccessView = get("/api/v1/app/access")
    suspend fun remote(): RemoteView = get("/api/v1/app/access/remote")

    suspend fun decideApproval(id: String, decision: String) {
        send("POST", "/api/v1/app/approvals/${enc(id)}", buildJsonObject { put("decision", decision) })
    }

    suspend fun decideAccess(id: String, approve: Boolean, scope: String?) {
        val body = buildJsonObject {
            put("decision", if (approve) "approve" else "deny")
            if (approve && scope != null) put("scope", scope)
        }
        send("POST", "/api/v1/app/access/requests/${enc(id)}", body)
    }

    suspend fun decideRemote(
        node: String,
        requestId: String,
        opHash: String,
        approve: Boolean,
        scope: String?,
        code: String,
    ) {
        val body = buildJsonObject {
            put("node", node)
            put("request", requestId)
            put("op_hash", opHash)
            put("decision", if (approve) "approve" else "deny")
            if (approve && scope != null) put("scope", scope) else put("scope", JsonPrimitive(null))
            put("auth", buildJsonObject { put("code", code) })
        }
        send("POST", "/api/v1/app/access/remote/decide", body)
    }

    suspend fun setAccessModes(node: String, modes: Map<String, String>) {
        val body = JsonObject(modes.mapValues { JsonPrimitive(it.value) })
        send("PUT", "/api/v1/app/access/${enc(node)}", body)
    }

    suspend fun renameDevice(node: String, name: String) {
        send("PUT", "/api/v1/app/devices/${enc(node)}", buildJsonObject { put("name", name) })
    }

    suspend fun pairAccept(link: String): PairAccepted =
        post("/api/v1/app/pair/accept", buildJsonObject { put("link", link) })

    suspend fun pairConfirm(flowId: String, matches: Boolean) {
        send("POST", "/api/v1/app/pair/${enc(flowId)}/confirm", buildJsonObject { put("match", matches) })
    }

    private suspend inline fun <reified T> get(path: String): T = api("GET", path, null)

    private suspend inline fun <reified T> post(path: String, body: JsonObject): T = api("POST", path, body)

    private suspend fun send(method: String, path: String, body: JsonObject) {
        raw(method, path, body)
    }

    private suspend inline fun <reified T> api(method: String, path: String, body: JsonObject?): T {
        val text = raw(method, path, body)
        if (text.isBlank() || text == "null") {
            @Suppress("UNCHECKED_CAST")
            return when (T::class) {
                Unit::class -> Unit as T
                List::class -> emptyList<Any>() as T
                else -> json.decodeFromString<T>("{}")
            }
        }
        if (T::class == Unit::class) {
            @Suppress("UNCHECKED_CAST")
            return Unit as T
        }
        return json.decodeFromString(text)
    }

    private suspend fun raw(method: String, path: String, body: JsonObject?): String = withContext(Dispatchers.IO) {
        val origin = base ?: throw ApiException("尚未登录节点")
        val url = origin.newBuilder().encodedPath(path).query(null).fragment(null).build()
        if (!isLoopback(url)) throw ApiException("拒绝非本机节点地址")
        val media = "application/json; charset=utf-8".toMediaType()
        val reqBody = body?.let { json.encodeToString(JsonObject.serializer(), it).toRequestBody(media) }
        val builder = Request.Builder().url(url)
            .header("Accept", "application/json")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("X-Workspace-Csrf", csrf)
        if (reqBody != null) builder.method(method, reqBody) else builder.method(method, null)
        val response = request(builder.build(), signed = true)
        val text = response.body?.string().orEmpty()
        val status = response.code
        response.close()
        if (status in 200..299) return@withContext text
        val err = runCatching { json.parseToJsonElement(text) }.getOrNull() as? JsonObject
        val message = listOf("message", "error", "code")
            .mapNotNull { (err?.get(it) as? JsonPrimitive)?.content }
            .firstOrNull()
            ?: "请求失败 ($status)"
        val code = (err?.get("code") as? JsonPrimitive)?.content
            ?: (err?.get("error") as? JsonPrimitive)?.content
            ?: ""
        throw ApiException(message, status, code)
    }

    private fun request(request: Request, signed: Boolean): okhttp3.Response {
        if (!isLoopback(request.url)) throw ApiException("拒绝非本机节点地址")
        val response = try {
            http.newCall(request).execute()
        } catch (e: IOException) {
            throw ApiException(e.message ?: "无法连接本机节点")
        }
        if (signed && response.code == 401) {
            response.close()
            _ready.value = false
            throw ApiException("登录已过期，正在重新连接", 401)
        }
        return response
    }

    private fun enc(value: String) = Uri.encode(value)

    companion object {
        fun isLoopback(url: HttpUrl): Boolean =
            url.scheme == "http" && (url.host == "127.0.0.1" || url.host == "localhost") &&
                url.port in 1..65535
    }
}

private class MemoryCookieJar : CookieJar {
    private val stored = ConcurrentHashMap<String, List<Cookie>>()
    override fun loadForRequest(url: HttpUrl): List<Cookie> =
        if (NodeClient.isLoopback(url)) stored[url.host].orEmpty() else emptyList()

    override fun saveFromResponse(url: HttpUrl, cookies: List<Cookie>) {
        if (NodeClient.isLoopback(url) && cookies.isNotEmpty()) stored[url.host] = cookies
    }

    fun clear() = stored.clear()
}

object NodeUrl {
    fun isLocal(value: String): Boolean = runCatching {
        val uri = Uri.parse(value)
        uri.scheme == "http" && (uri.host == "127.0.0.1" || uri.host == "localhost") &&
            uri.port in 1..65535 && uri.userInfo == null
    }.getOrDefault(false)
}
