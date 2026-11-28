package com.cranibox.android.api

import kotlin.test.Test
import kotlin.test.assertEquals

class CopyTest {
    @Test fun defaultModesAllowMessagesOnly() {
        val modes = Copy.defaultModes()
        assertEquals("allow", modes["messages"])
        assertEquals("off", modes["terminals"])
        assertEquals("off", modes["computer"])
    }

    @Test fun approvalTitles() {
        assertEquals("核对新设备", Copy.approvalTitle("pairing"))
        assertEquals("需要你确认", Copy.approvalTitle("unknown"))
        assertEquals("远程桌面", Copy.capabilityName("desktop"))
        assertEquals("运行命令", Copy.operationName("exec.run"))
    }
}
