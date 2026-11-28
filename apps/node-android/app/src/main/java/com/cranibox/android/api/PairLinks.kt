package com.cranibox.android.api

/**
 * Pairing tickets are `cranibox://pair/<payload>`. Public `/pair#…` https links
 * from a node's invite page are also accepted as a pasted fallback.
 */
object PairLinks {
    const val PREFIX = "cranibox://pair/"

    fun parse(raw: String): String? {
        val text = raw.trim().removeSurrounding("\"")
        if (text.isEmpty()) return null
        if (text.startsWith(PREFIX, ignoreCase = true) && text.length > PREFIX.length) {
            val payload = text.substring(PREFIX.length)
            if (payload.isBlank() || payload == "/") return null
            return PREFIX + payload
        }
        val uri = runCatching { java.net.URI(text) }.getOrNull() ?: return null
        val scheme = uri.scheme?.lowercase() ?: return null
        val path = uri.path.orEmpty()
        val host = uri.host.orEmpty()
        if (scheme == "cranibox" && (host.equals("pair", ignoreCase = true) || path == "/pair" || path.startsWith("/pair/"))) {
            val payload = when {
                host.equals("pair", ignoreCase = true) -> path.trim('/').ifBlank { uri.fragment.orEmpty() }
                path.startsWith("/pair/") -> path.removePrefix("/pair/").trim('/').ifBlank { uri.fragment.orEmpty() }
                else -> uri.fragment.orEmpty()
            }
            if (payload.isBlank()) return null
            return text
        }
        if (scheme == "http" || scheme == "https") {
            if (path == "/pair" || path.startsWith("/pair/")) {
                val payload = when {
                    path.startsWith("/pair/") -> path.removePrefix("/pair/").trim('/')
                    else -> ""
                }.ifBlank { uri.fragment.orEmpty() }.ifBlank { uri.query.orEmpty() }
                if (payload.isBlank()) return null
                return text
            }
        }
        return null
    }
}
