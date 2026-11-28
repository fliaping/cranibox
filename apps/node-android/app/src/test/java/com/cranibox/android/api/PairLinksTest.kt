package com.cranibox.android.api

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class PairLinksTest {
    @Test fun acceptsCraniboxPrefix() {
        val link = "cranibox://pair/ABCDEF"
        assertEquals(link, PairLinks.parse("  $link  "))
    }

    @Test fun acceptsHttpsPairPath() {
        val link = "https://example.com/pair#ticket"
        assertEquals(link, PairLinks.parse(link))
    }

    @Test fun rejectsEmptyAndUnrelated() {
        assertNull(PairLinks.parse(""))
        assertNull(PairLinks.parse("https://example.com/other"))
        assertNull(PairLinks.parse("cranibox://pair/"))
        assertNull(PairLinks.parse("javascript:alert(1)"))
    }
}
