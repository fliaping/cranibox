package com.cranibox.android.ui.phone

import android.app.AlertDialog
import android.content.ComponentName
import android.content.Intent
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.produceState
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import kotlinx.coroutines.delay
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.cranibox.android.AgentOverlayService
import com.cranibox.android.AgentScreenView
import com.cranibox.android.NodeService
import com.cranibox.android.R
import com.cranibox.android.node

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PreviewScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val app = context.node
    val connected by app.phone.connected.collectAsState()
    val available by AgentOverlayService.available.collectAsState()
    val requested by AgentOverlayService.requested.collectAsState()
    val paused by produceState(initialValue = app.bridge.paused, key1 = app) { while (true) { value = app.bridge.paused; delay(500) } }
    DisposableEffect(Unit) {
        app.agentScreenVisible.value = true
        onDispose { app.agentScreenVisible.value = false }
    }
    BackHandler(onBack = onBack)
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(stringResource(R.string.preview_title)) },
            navigationIcon = {
                IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = "返回") }
            },
        )
        Text(
            when {
                !connected -> "手机控制未连接"
                paused -> "Agent 已暂停 · 画面仍可查看"
                else -> "手机控制已连接 · 实时画面"
            },
            modifier = Modifier.padding(horizontal = 16.dp),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(stringResource(R.string.preview_readonly), modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        AndroidView(factory = { AgentScreenView(it) }, modifier = Modifier.fillMaxWidth().weight(1f).padding(16.dp))
        Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = {
                if (available) {
                    AgentOverlayService.setRequested(context, !requested)
                } else {
                    AlertDialog.Builder(context).setTitle("开启 Agent 屏幕悬浮窗")
                        .setMessage("请在下一页启用「Cranibox · Agent 屏幕悬浮窗」。此无障碍服务只展示独立屏幕画面，不读取其他应用内容，也不执行触摸。")
                        .setPositiveButton("前往开启") { _, _ ->
                            AgentOverlayService.setRequested(context, true)
                            val details = Intent("android.settings.ACCESSIBILITY_DETAILS_SETTINGS")
                                .putExtra(Intent.EXTRA_COMPONENT_NAME, ComponentName(context, AgentOverlayService::class.java))
                            runCatching { context.startActivity(details) }
                                .onFailure { context.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }
                        }.setNegativeButton("取消", null).show()
                }
            }, modifier = Modifier.weight(1f)) {
                Text(
                    when {
                        !available -> "开启跨应用悬浮窗"
                        !requested -> "显示悬浮窗"
                        else -> "关闭悬浮窗"
                    },
                )
            }
            Button(onClick = {
                context.startForegroundService(
                    Intent(context, NodeService::class.java).setAction(if (paused) NodeService.RESUME else NodeService.EMERGENCY),
                )
            }, modifier = Modifier.weight(1f)) {
                Text(if (paused) stringResource(R.string.emergency_resume) else stringResource(R.string.emergency_stop))
            }
        }
    }
}
