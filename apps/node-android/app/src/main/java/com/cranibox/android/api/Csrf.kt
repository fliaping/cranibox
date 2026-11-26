package com.cranibox.android.api

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

object Csrf {
    private val json = Json { ignoreUnknownKeys = true }

    fun parse(html: String): String? {
        val config = html.substringAfter("window.workspaceWeb=", missingDelimiterValue = "")
            .substringBefore(";</script>", missingDelimiterValue = "")
            .trim()
        if (config.isEmpty() || !config.startsWith("{")) return null
        return runCatching {
            json.parseToJsonElement(config).jsonObject["csrf"]?.jsonPrimitive?.content
                ?.takeIf { it.isNotEmpty() }
        }.getOrNull()
    }
}
