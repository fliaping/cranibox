package com.cranibox.android.pico

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import java.io.File
import java.security.SecureRandom

/**
 * Generates a PicoClaw v0.3.1 (config schema 2) home directory for the Android
 * node. Paths and the Pico WebSocket port were verified against the pinned
 * PicoClaw sources (`pkg/config/defaults.go`, `pkg/channels/pico/pico.go`).
 */
object PicoClawConfig {
    const val VERSION = 2
    const val PINNED_TAG = "v0.3.1"
    const val PINNED_COMMIT = "2cf030d2fd3b871d7ec17e3be34c24688aac76da"
    const val GATEWAY_MODEL = "cranibox-llm"
    const val CUSTOM_MODEL = "custom"
    const val LOCAL_GATEWAY = "http://127.0.0.1:3424/v1"

    private val json = Json { prettyPrint = true; encodeDefaults = true }

    data class Paths(
        val home: File,
        val craniboxExecutable: File,
        val craniboxConfig: File,
        val craniboxData: File,
        val filesDir: File,
        val cacheDir: File,
    ) {
        val configFile: File get() = File(home, "config.json")
        val securityFile: File get() = File(home, ".security.yml")
        val workspace: File get() = File(home, "workspace")
        val tokenFile: File get() = File(home, "channel.token")
    }

    @Serializable
    data class ModelSettings(
        val useLocalGateway: Boolean = true,
        val customApiBase: String = "",
        val customApiKey: String = "",
        val customModel: String = "gpt-4o-mini",
    ) {
        val customReady: Boolean
            get() = customApiBase.trim().isNotEmpty() && customModel.trim().isNotEmpty()
        val hasAnyModel: Boolean
            get() = useLocalGateway || customReady
        val defaultModelName: String
            get() = when {
                useLocalGateway -> GATEWAY_MODEL
                customReady -> CUSTOM_MODEL
                else -> GATEWAY_MODEL
            }
    }

    data class Generated(val token: String, val settings: ModelSettings)

    fun generateToken(): String {
        val bytes = ByteArray(32)
        SecureRandom().nextBytes(bytes)
        return bytes.joinToString("") { b -> "%02x".format(b) }
    }

    fun readOrCreateToken(paths: Paths): String {
        val existing = paths.tokenFile.takeIf { it.isFile }?.readText()?.trim().orEmpty()
        if (existing.length >= 16) return existing
        val token = generateToken()
        paths.home.mkdirs()
        paths.tokenFile.writeText(token)
        paths.tokenFile.setReadable(false, false)
        paths.tokenFile.setReadable(true, true)
        paths.tokenFile.setWritable(false, false)
        paths.tokenFile.setWritable(true, true)
        return token
    }

    fun write(paths: Paths, settings: ModelSettings, token: String = readOrCreateToken(paths)): Generated {
        require(paths.craniboxExecutable.path.isNotBlank())
        paths.home.mkdirs()
        paths.workspace.mkdirs()
        val defaultName = if (settings.hasAnyModel) settings.defaultModelName else GATEWAY_MODEL
        val models = buildList {
            if (settings.useLocalGateway) {
                add(
                    ModelEntry(
                        modelName = GATEWAY_MODEL,
                        model = "openai/cranibox",
                        apiBase = LOCAL_GATEWAY,
                        enabled = true,
                    ),
                )
            }
            if (settings.customReady) {
                add(
                    ModelEntry(
                        modelName = CUSTOM_MODEL,
                        model = "openai/${settings.customModel.trim()}",
                        apiBase = settings.customApiBase.trim().trimEnd('/'),
                        enabled = true,
                    ),
                )
            }
        }
        val config = ConfigFile(
            version = VERSION,
            agents = Agents(defaults = AgentDefaults(modelName = defaultName)),
            modelList = models,
            gateway = Gateway(host = PicoProtocol.DEFAULT_HOST, port = PicoProtocol.DEFAULT_PORT),
            channels = Channels(pico = PicoChannel(enabled = true, allowTokenQuery = false)),
            tools = Tools(
                exec = ExecTool(enabled = false),
                mcp = McpTool(
                    enabled = true,
                    servers = mcpServers(paths),
                ),
            ),
        )
        paths.configFile.writeText(json.encodeToString(ConfigFile.serializer(), config))
        paths.securityFile.writeText(securityYaml(token, settings))
        paths.securityFile.setReadable(false, false)
        paths.securityFile.setReadable(true, true)
        paths.securityFile.setWritable(false, false)
        paths.securityFile.setWritable(true, true)
        if (!paths.tokenFile.exists() || paths.tokenFile.readText().trim() != token) {
            paths.tokenFile.writeText(token)
        }
        paths.tokenFile.setReadable(false, false)
        paths.tokenFile.setReadable(true, true)
        paths.tokenFile.setWritable(false, false)
        paths.tokenFile.setWritable(true, true)
        return Generated(token = token, settings = settings)
    }

    fun mcpServers(paths: Paths): Map<String, McpServer> {
        val exe = paths.craniboxExecutable.absolutePath
        val config = paths.craniboxConfig.absolutePath
        val data = paths.craniboxData.absolutePath
        val env = mapOf(
            "HOME" to paths.filesDir.absolutePath,
            "XDG_CONFIG_HOME" to File(paths.filesDir, ".config").absolutePath,
            "TMPDIR" to paths.cacheDir.absolutePath,
        )
        return mapOf(
            "cranibox" to McpServer(
                enabled = true,
                type = "stdio",
                command = exe,
                args = listOf("--config", config, "mcp"),
                env = env,
            ),
            "cranibox-devices" to McpServer(
                enabled = true,
                type = "stdio",
                command = exe,
                args = listOf("--config", config, "app-mcp", "--data", data, "--service", "devices"),
                env = env,
            ),
            "cranibox-computer" to McpServer(
                enabled = true,
                type = "stdio",
                command = exe,
                args = listOf("--config", config, "app-mcp", "--data", data, "--service", "computer"),
                env = env,
            ),
        )
    }

    fun securityYaml(token: String, settings: ModelSettings): String = buildString {
        appendLine("channels:")
        appendLine("  pico:")
        appendLine("    token: \"$token\"")
        appendLine("model_list:")
        if (settings.useLocalGateway) {
            appendLine("  $GATEWAY_MODEL:0:")
            appendLine("    api_keys:")
            appendLine("      - \"local\"")
        }
        if (settings.customReady) {
            val key = settings.customApiKey.ifBlank { "local" }.replace("\"", "\\\"")
            appendLine("  $CUSTOM_MODEL:0:")
            appendLine("    api_keys:")
            appendLine("      - \"$key\"")
        }
    }

    @Serializable data class ConfigFile(
        val version: Int,
        val agents: Agents,
        @SerialName("model_list") val modelList: List<ModelEntry>,
        val gateway: Gateway,
        val channels: Channels,
        val tools: Tools,
    )
    @Serializable data class Agents(val defaults: AgentDefaults)
    @Serializable data class AgentDefaults(@SerialName("model_name") val modelName: String)
    @Serializable data class ModelEntry(
        @SerialName("model_name") val modelName: String,
        val model: String,
        @SerialName("api_base") val apiBase: String,
        val enabled: Boolean = true,
    )
    @Serializable data class Gateway(val host: String, val port: Int)
    @Serializable data class Channels(val pico: PicoChannel)
    @Serializable data class PicoChannel(val enabled: Boolean, @SerialName("allow_token_query") val allowTokenQuery: Boolean)
    @Serializable data class Tools(val exec: ExecTool, val mcp: McpTool)
    @Serializable data class ExecTool(val enabled: Boolean)
    @Serializable data class McpTool(val enabled: Boolean, val servers: Map<String, McpServer>)
    @Serializable data class McpServer(
        val enabled: Boolean,
        val type: String,
        val command: String,
        val args: List<String>,
        val env: Map<String, String>,
    )
}
