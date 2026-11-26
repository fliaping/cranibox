package com.cranibox.android

import com.cranibox.android.core.adb.OwnListeningPorts
import com.cranibox.android.core.bridge.BridgeToken
import com.cranibox.android.core.bridge.LoopbackHttpServer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.util.concurrent.TimeUnit

/** The executable is extracted by PackageManager; never executed from a writable app directory. */
class NodeRuntime(private val app: NodeApp) {
    private val config = File(app.filesDir, ".config/cranibox/settings.json")
    private val executable = File(app.applicationInfo.nativeLibraryDir, "libcranibox.so")
    private val log = File(app.filesDir, "node.log")
    @Volatile private var process: Process? = null
    @Volatile private var server: LoopbackHttpServer? = null

    private fun builder(vararg args: String): ProcessBuilder = ProcessBuilder(
        listOf(executable.path, "--config", config.path) + args,
    ).directory(app.filesDir).apply {
        environment().apply {
            put("HOME", app.filesDir.path)
            put("XDG_CONFIG_HOME", File(app.filesDir, ".config").path)
            put("TMPDIR", app.cacheDir.path)
            put("SHELL", "/system/bin/sh")
            put("PATH", "/system/bin:/system/xbin")
        }
    }

    @Synchronized
    private fun command(vararg args: String): String {
        val output = File(app.cacheDir, "node-command.out")
        val child = builder(*args).redirectErrorStream(true).redirectOutput(output).start()
        try {
            check(child.waitFor(5, TimeUnit.SECONDS)) { "节点命令超时" }
            if (child.exitValue() != 0) {
                if (log.length() > 1024 * 1024) log.writeText("")
                log.appendText("节点命令 ${args.firstOrNull().orEmpty()} 失败（${child.exitValue()}）\n" +
                    output.readText().takeLast(6_000) + "\n")
                error("节点命令失败，请查看节点日志")
            }
            return output.readText().trim()
        } finally {
            child.destroyForcibly()
            output.delete()
        }
    }

    /** Owner actions update both the bridge and the node's persistent stop/lease fence. */
    fun setComputerPaused(paused: Boolean) {
        val action = if (paused) "emergency_stop" else "resume"
        command("peer", "computer-owner", "--command", "{\"action\":\"$action\"}")
    }

    suspend fun run(): Unit = withContext(Dispatchers.IO) {
        check(executable.canExecute()) { "APK 缺少可执行的手机节点，请先运行 build-native.sh 再打包" }
        val tokenFile = File(app.filesDir, "bridge.token")
        val token = BridgeToken.generate()
        tokenFile.writeText(token)
        tokenFile.setReadable(false, false); tokenFile.setReadable(true, true)
        val bridge = LoopbackHttpServer(0, token, app.bridge)
        bridge.start()
        server = bridge
        OwnListeningPorts.add(bridge.port)
        try {
            command("peer", "computer", "--address", "127.0.0.1:${bridge.port}", "--token-file", tokenFile.path)
            // Bound the persistent log across service restarts; no login tokens are written here.
            if (log.length() > 1024 * 1024) log.writeText("")
            val child = builder("serve").redirectErrorStream(true).redirectOutput(ProcessBuilder.Redirect.appendTo(log)).start()
            process = child
            app.status.value = "节点启动中…"
            var ready = false
            for (attempt in 0 until 30) {
                check(child.isAlive) { "节点启动失败，请查看节点日志" }
                val url = runCatching { command("ui-url") }.getOrNull()
                if (url != null && NodeUrl.isLocal(url)) {
                    OwnListeningPorts.add(android.net.Uri.parse(url).port)
                    app.loginUrl.value = url
                    app.status.value = "节点运行中"
                    ready = true
                    app.work.launch { app.pico.start() }
                    break
                }
                delay(500)
            }
            check(ready) { "等待节点界面超时，请查看节点日志" }
            while (child.isAlive) delay(1_000)
            error("节点已退出（${child.exitValue()}），可重新启动")
        } finally { close() }
    }

    @Synchronized
    fun close() {
        app.pico.stop()
        app.loginUrl.value?.let { OwnListeningPorts.remove(android.net.Uri.parse(it).port) }
        app.loginUrl.value = null
        process?.let { child ->
            child.destroy()
            if (!child.waitFor(2, TimeUnit.SECONDS)) child.destroyForcibly()
        }
        process = null
        server?.let { OwnListeningPorts.remove(it.port); it.close() }
        server = null
    }
}

object NodeUrl {
    fun isLocal(value: String): Boolean = runCatching {
        val uri = android.net.Uri.parse(value)
        uri.scheme == "http" && uri.host == "127.0.0.1" && uri.port in 1..65535 && uri.userInfo == null
    }.getOrDefault(false)

    fun sameOrigin(value: String, base: String): Boolean = isLocal(value) && runCatching {
        android.net.Uri.parse(value).port == android.net.Uri.parse(base).port
    }.getOrDefault(false)
}
