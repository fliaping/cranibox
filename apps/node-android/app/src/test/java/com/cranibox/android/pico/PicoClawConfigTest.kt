package com.cranibox.android.pico

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.test.Test
import kotlin.test.assertContains
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class PicoClawConfigTest {
    private fun paths(root: File) = PicoClawConfig.Paths(
        home = File(root, "picoclaw"),
        craniboxExecutable = File(root, "lib/libcranibox.so"),
        craniboxConfig = File(root, "cfg/settings.json"),
        craniboxData = File(root, "cfg"),
        filesDir = root,
        cacheDir = File(root, "cache"),
    )


    private fun assertStreamingEnabled(config: String) {
        val root = Json.parseToJsonElement(config).jsonObject
        val models = root.getValue("model_list").jsonArray
        assertTrue(models.isNotEmpty())
        for (model in models) {
            val streaming = model.jsonObject.getValue("streaming").jsonObject
            assertTrue(streaming.getValue("enabled").jsonPrimitive.boolean, "model streaming.enabled")
        }
        val pico = root.getValue("channels").jsonObject.getValue("pico").jsonObject
        val channelStreaming = pico.getValue("streaming").jsonObject
        assertTrue(channelStreaming.getValue("enabled").jsonPrimitive.boolean, "channel streaming.enabled")
        assertEquals(0, channelStreaming.getValue("throttle_seconds").jsonPrimitive.int)
        assertEquals(1, channelStreaming.getValue("min_growth_chars").jsonPrimitive.int)
    }

    @Test fun writesSchemaV2LoopbackGatewayAndStdioMcp() {
        val root = File.createTempFile("pico-cfg", "dir").apply { delete(); mkdirs() }
        try {
            val settings = PicoClawConfig.ModelSettings(useLocalGateway = true)
            val generated = PicoClawConfig.write(paths(root), settings, token = "tok-1234567890abcdef")
            assertEquals("tok-1234567890abcdef", generated.token)
            val text = File(root, "picoclaw/config.json").readText()
            assertStreamingEnabled(text)
            assertContains(text, """"version": 2""")
            assertContains(text, """"host": "127.0.0.1"""")
            assertContains(text, """"port": 18790""")
            assertContains(text, "http://127.0.0.1:3424/v1")
            assertContains(text, """"model_name": "cranibox-llm"""")
            assertContains(text, """"allow_token_query": false""")
            assertContains(text, """"enabled": false""") // exec tool
            assertContains(text, """"--config"""")
            assertContains(text, """"mcp"""")
            assertContains(text, "app-mcp")
            assertContains(text, "cranibox-computer")
            assertContains(text, "cranibox-devices")
            val security = File(root, "picoclaw/.security.yml").readText()
            assertContains(security, "tok-1234567890abcdef")
            assertContains(security, "cranibox-llm:0:")
            assertFalse(text.contains("0.0.0.0"))
        } finally {
            root.deleteRecursively()
        }
    }

    @Test fun customEndpointBecomesDefaultModel() {
        val root = File.createTempFile("pico-cfg", "dir").apply { delete(); mkdirs() }
        try {
            val settings = PicoClawConfig.ModelSettings(
                useLocalGateway = false,
                customApiBase = "https://api.example.com/v1",
                customApiKey = "sk-test",
                customModel = "gpt-4.1",
            )
            PicoClawConfig.write(paths(root), settings, token = "tok-1234567890abcdef")
            val text = File(root, "picoclaw/config.json").readText()
            assertStreamingEnabled(text)
            assertContains(text, """"model_name": "custom"""")
            assertContains(text, "openai/gpt-4.1")
            assertContains(text, "https://api.example.com/v1")
            assertFalse(text.contains("127.0.0.1:3424"))
            val security = File(root, "picoclaw/.security.yml").readText()
            assertContains(security, "sk-test")
        } finally {
            root.deleteRecursively()
        }
    }

    @Test fun rejectsBlankExecutableIsNotRequiredAtWriteIfPathPresent() {
        val settings = PicoClawConfig.ModelSettings()
        assertTrue(settings.hasAnyModel)
        assertEquals(PicoClawConfig.GATEWAY_MODEL, PicoClawConfig.ModelSettings().defaultModelName)
        assertEquals(PicoClawConfig.CUSTOM_MODEL, PicoClawConfig.ModelSettings(useLocalGateway = false, customApiBase = "http://x/v1", customModel = "m").defaultModelName)
    }
}
