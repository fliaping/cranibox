package com.cranibox.android

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.mutableStateOf
import com.cranibox.android.api.PairLinks
import com.cranibox.android.ui.CraniboxApp
import com.cranibox.android.ui.theme.CraniboxTheme

class MainActivity : ComponentActivity() {
    private val pendingLink = mutableStateOf<String?>(null)
    private val openApprovals = mutableStateOf(false)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        handle(intent)
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        startForegroundService(Intent(this, NodeService::class.java))
        setContent {
            CraniboxTheme {
                CraniboxApp(
                    pendingLink = pendingLink.value,
                    onPendingLinkConsumed = { pendingLink.value = null },
                    openApprovals = openApprovals.value,
                    onApprovalsOpened = { openApprovals.value = false },
                )
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handle(intent)
    }

    private fun handle(intent: Intent?) {
        when {
            intent?.action == ACTION_APPROVALS -> openApprovals.value = true
            else -> PairLinks.parse(intent?.data?.toString().orEmpty())?.let { pendingLink.value = it }
        }
    }

    companion object {
        const val ACTION_APPROVALS = "com.cranibox.node.OPEN_APPROVALS"
    }
}
