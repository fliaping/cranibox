package com.cranibox.android.ui.phone

import android.app.Application
import android.app.AlertDialog
import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import com.cranibox.android.AgentScreenView
import com.cranibox.android.NodeApp
import com.cranibox.android.NodeService
import com.cranibox.android.R
import com.cranibox.android.StatusTone
import com.cranibox.android.Tone

class PhoneViewModel(application: Application) : AndroidViewModel(application) {
    val app = application as NodeApp
    val paused = MutableStateFlow(app.bridge.paused)
    init {
        viewModelScope.launch {
            while (true) {
                paused.value = app.bridge.paused
                delay(500)
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PhoneScreen(onPreview: () -> Unit, snackbar: SnackbarHostState, model: PhoneViewModel = viewModel()) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val status by model.app.status.collectAsState()
    val phone by model.app.adb.state.collectAsState()
    val busy by model.app.adb.busy.collectAsState()
    val connected by model.app.phone.connected.collectAsState()
    val paused by model.paused.collectAsState()
    var code by remember { mutableStateOf("") }
    var pairPort by remember { mutableStateOf("") }
    var connectPort by remember { mutableStateOf("") }
    val nodeOn = status == "节点运行中" || status.endsWith("…")
    fun service(action: String? = null, pairCode: String? = null, port: Int? = null) {
        context.startForegroundService(Intent(context, NodeService::class.java).apply {
            this.action = action
            pairCode?.let { putExtra(NodeService.CODE, it) }
            port?.let { putExtra(NodeService.PORT, it) }
        })
    }
    fun parsePort(raw: String): Int? {
        val value = raw.trim()
        if (value.isEmpty()) return null
        val port = value.toIntOrNull()
        return port?.takeIf { it in 1..65535 }
    }
    Column(Modifier.fillMaxSize()) {
        TopAppBar(title = { Text(stringResource(R.string.tab_phone)) })
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.primaryContainer)) {
                Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
                    Column(Modifier.weight(1f)) {
                        Text("节点", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onPrimaryContainer)
                        Text(status, color = MaterialTheme.colorScheme.onPrimaryContainer)
                    }
                    Switch(
                        checked = nodeOn && StatusTone.node(status) != Tone.OFF,
                        onCheckedChange = { on ->
                            if (on) service()
                            else AlertDialog.Builder(context)
                                .setTitle(context.getString(R.string.stop_node))
                                .setMessage(context.getString(R.string.stop_node_confirm))
                                .setPositiveButton(context.getString(R.string.stop_node)) { _, _ -> service(NodeService.STOP) }
                                .setNegativeButton(context.getString(R.string.cancel), null)
                                .show()
                        },
                    )
                }
            }
            Card(colors = CardDefaults.cardColors(containerColor = if (paused) MaterialTheme.colorScheme.errorContainer else MaterialTheme.colorScheme.surfaceContainer)) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(if (paused) "Agent 已暂停" else if (connected) "Agent 控制已就绪" else "Agent 控制尚未就绪", style = MaterialTheme.typography.titleMedium)
                    Text(
                        if (paused) "远端 Agent 无法操作这台手机，需在本机恢复。"
                        else if (!nodeOn) "先启动节点，再连接手机控制。"
                        else if (!connected) "完成下方的无线调试配对后，仍需节点授权与租约。"
                        else "通知栏也可随时停止。",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Button(onClick = { service(if (paused) NodeService.RESUME else NodeService.EMERGENCY) }) {
                        Text(if (paused) stringResource(R.string.emergency_resume) else stringResource(R.string.emergency_stop))
                    }
                }
            }
            Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer)) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(stringResource(R.string.preview_title), style = MaterialTheme.typography.titleMedium)
                    Text(stringResource(R.string.preview_readonly), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    AndroidView(
                        factory = { AgentScreenView(it) },
                        modifier = Modifier.fillMaxWidth().height(220.dp),
                    )
                    FilledTonalButton(onClick = onPreview, modifier = Modifier.fillMaxWidth()) { Text("查看完整画面") }
                }
            }
            Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer)) {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Text(stringResource(R.string.wireless_pair), style = MaterialTheme.typography.titleMedium)
                    Text(phone, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    if (busy) LinearProgressIndicator(Modifier.fillMaxWidth())
                    Text("1. 在开发者选项中开启「无线调试」\n2. 选择「使用配对码配对设备」，保持弹窗打开\n3. 把六位配对码填在下方，或从通知栏输入", color = MaterialTheme.colorScheme.onSurfaceVariant)
                    FilledTonalButton(onClick = {
                        service(NodeService.SHOW_PAIR)
                        runCatching { context.startActivity(Intent("android.settings.WIRELESS_DEBUGGING_SETTINGS")) }
                            .onFailure { context.startActivity(Intent(Settings.ACTION_APPLICATION_DEVELOPMENT_SETTINGS)) }
                    }, modifier = Modifier.fillMaxWidth()) { Text("打开无线调试") }
                    OutlinedTextField(value = code, onValueChange = { if (it.length <= 6) code = it.filter(Char::isDigit) }, label = { Text("六位配对码") }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !busy)
                    OutlinedTextField(value = pairPort, onValueChange = { pairPort = it.filter(Char::isDigit).take(5) }, label = { Text("配对端口 · 选填") }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !busy)
                    Button(onClick = {
                        if (code.length != 6) scope.launch { snackbar.showSnackbar("请输入六位配对码") }
                        else {
                            val port = parsePort(pairPort)
                            if (pairPort.isNotBlank() && port == null) scope.launch { snackbar.showSnackbar("端口需在 1–65535 之间") }
                            else { service(NodeService.PAIR, code, port); code = "" }
                        }
                    }, modifier = Modifier.fillMaxWidth(), enabled = !busy) { Text("配对并启动") }
                    OutlinedTextField(value = connectPort, onValueChange = { connectPort = it.filter(Char::isDigit).take(5) }, label = { Text("连接端口 · 选填") }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number), singleLine = true, modifier = Modifier.fillMaxWidth(), enabled = !busy)
                    OutlinedButton(onClick = {
                        val port = parsePort(connectPort)
                        if (connectPort.isNotBlank() && port == null) scope.launch { snackbar.showSnackbar("端口需在 1–65535 之间") }
                        else service(NodeService.CONNECT, port = port)
                    }, modifier = Modifier.fillMaxWidth(), enabled = !busy) { Text("重新连接") }
                    TextButton(onClick = {
                        AlertDialog.Builder(context)
                            .setTitle("清除无线调试配对？")
                            .setMessage("清除 Cranibox 的调试钥匙并停止手机控制，之后需要重新配对。")
                            .setPositiveButton("清除") { _, _ -> service(NodeService.FORGET) }
                            .setNegativeButton(context.getString(R.string.cancel), null)
                            .show()
                    }) { Text("清除配对") }
                }
            }
        }
    }
}
