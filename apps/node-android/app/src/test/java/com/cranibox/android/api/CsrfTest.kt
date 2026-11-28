package com.cranibox.android.api

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class CsrfTest {
    @Test fun readsTokenFromPage() {
        val html = """<script>window.workspaceWeb={"csrf":"secret-token","language":"zh-cn"};</script>"""
        assertEquals("secret-token", Csrf.parse(html))
    }

    @Test fun ignoresMissingOrEmpty() {
        assertNull(Csrf.parse("<html></html>"))
        assertNull(Csrf.parse("""<script>window.workspaceWeb={"csrf":""};</script>"""))
    }
}
