package com.cranibox.android.pico

import android.content.Context
import com.cranibox.android.NodeApp
import com.cranibox.android.core.adb.OwnListeningPorts
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.TimeUnit

/** Supervises `libpicoclaw.so gateway` next to the Cranibox node. */
class PicoClawRuntime(private val app: NodeApp) {
    private val prefs = app.getSharedPreferences("picoclaw-settings", Context.MODE_PRIVATE)
    private val log = File(app.filesDir, "picoclaw.log")
    private val executable = File(app.applicationInfo.nativeLibraryDir, "libpicoclaw.so")
    private val _ready = MutableStateFlow(false)
    val ready = _ready.asStateFlow()
    val status = MutableStateFlow("Agent 尚未启动")
    @Volatile private var process: Process? = null
    @Volatile var token: String = ""
        private set
    @Volatile var settings: PicoClawConfig.ModelSettings = PicoClawConfig.ModelSettings()
        private set

    fun paths(): PicoClawConfig.Paths = PicoClawConfig.Paths(
        home = File(app.filesDir, "picoclaw"),
        craniboxExecutable = File(app.applicationInfo.nativeLibraryDir, "libcranibox.so"),
        craniboxConfig = File(app.filesDir, ".config/cranibox/settings.json"),
        craniboxData = File(app.filesDir, ".config/cranibox"),
        filesDir = app.filesDir,
        cacheDir = app.cacheDir,
    )

    fun loadSettings(): PicoClawConfig.ModelSettings = PicoClawConfig.ModelSettings(
        useLocalGateway = prefs.getBoolean("useLocalGateway", true),
        customApiBase = prefs.getString("customApiBase", "").orEmpty(),
        customApiKey = prefs.getString("customApiKey", "").orEmpty(),
        customModel = prefs.getString("customModel", "gpt-4o-mini").orEmpty().ifBlank { "gpt-4o-mini" },
    )

    fun saveSettings(next: PicoClawConfig.ModelSettings) {
        check(
            prefs.edit()
                .putBoolean("useLocalGateway", next.useLocalGateway)
                .putString("customApiBase", next.customApiBase)
                .putString("customApiKey", next.customApiKey)
                .putString("customModel", next.customModel)
                .commit(),
        ) { "cannot save Agent settings" }
        settings = next
    }

    fun prepareConfig(): PicoClawConfig.Generated {
        settings = loadSettings()
        val generated = PicoClawConfig.write(paths(), settings)
        token = generated.token
        return generated
    }

    suspend fun start(): Unit = withContext(Dispatchers.IO) {
        stopLocked()
        if (!executable.canExecute()) {
            status.value = "APK 缺少 libpicoclaw.so，请用 build-native.sh 重新打包"
            _ready.value = false
            return@withContext
        }
        val generated = prepareConfig()
        if (log.length() > 1024 * 1024) log.writeText("")
        OwnListeningPorts.add(PicoProtocol.DEFAULT_PORT)
        val builder = ProcessBuilder(
            executable.path, "gateway", "--host", PicoProtocol.DEFAULT_HOST, "--allow-empty",
        ).directory(paths().home).redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.appendTo(log))
        builder.environment().apply {
            put("HOME", app.filesDir.path)
            put("PICOCLAW_HOME", paths().home.path)
            put("TMPDIR", app.cacheDir.path)
            put("PATH", "/system/bin:/system/xbin")
            put("PICOCLAW_CHANNELS_PICO_TOKEN", generated.token)
        }
        val child = builder.start()
        process = child
        status.value = "Agent 启动中…"
        var up = false
        for (i in 0 until 40) {
            if (!child.isAlive) {
                status.value = "Agent 启动失败，请查看 Agent 日志"
                _ready.value = false
                return@withContext
            }
            if (portOpen()) {
                up = true
                break
            }
            delay(250)
        }
        if (!up) {
            status.value = "等待 Agent 端口超时"
            _ready.value = false
            return@withContext
        }
        _ready.value = true
        status.value = if (generated.settings.hasAnyModel) "Agent 已就绪" else "Agent 已启动，请先配置模型"
        while (child.isAlive) delay(1_000)
        _ready.value = false
        status.value = "Agent 已退出"
        OwnListeningPorts.remove(PicoProtocol.DEFAULT_PORT)
        process = null
    }

    fun stop() {
        stopLocked()
        _ready.value = false
        status.value = "Agent 已停止"
    }

    @Synchronized
    private fun stopLocked() {
        process?.let { child ->
            child.destroy()
            if (!child.waitFor(3, TimeUnit.SECONDS)) child.destroyForcibly()
        }
        process = null
        OwnListeningPorts.remove(PicoProtocol.DEFAULT_PORT)
        _ready.value = false
    }

    private fun portOpen(): Boolean = runCatching {
        Socket().use { socket ->
            socket.connect(InetSocketAddress(PicoProtocol.DEFAULT_HOST, PicoProtocol.DEFAULT_PORT), 200)
            true
        }
    }.getOrDefault(false)

    fun sessionsFile(): File = File(paths().home, "sessions.json")

    fun loadSessions(): List<ChatSession> {
        val file = sessionsFile()
        if (!file.isFile) return emptyList()
        return runCatching {
            val array = org.json.JSONArray(file.readText())
            buildList {
                for (i in 0 until array.length()) {
                    val o = array.getJSONObject(i)
                    add(ChatSession(o.getString("id"), o.optString("title"), o.optLong("updatedAt")))
                }
            }
        }.getOrDefault(emptyList())
    }

    fun saveSessions(sessions: List<ChatSession>) {
        val array = org.json.JSONArray()
        sessions.take(40).forEach { session ->
            array.put(JSONObject().put("id", session.id).put("title", session.title).put("updatedAt", session.updatedAt))
        }
        sessionsFile().writeText(array.toString())
    }
}
