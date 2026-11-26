package com.cranibox.android.ui.chat

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Send
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Stop
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SuggestionChip
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
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
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.viewmodel.compose.viewModel
import com.cranibox.android.R
import com.cranibox.android.pico.ChatMessage
import com.cranibox.android.pico.ChatSession
import com.cranibox.android.pico.ChatUiState
import com.cranibox.android.pico.PicoClawConfig
import com.cranibox.android.pico.PicoProtocol
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatScreen(
    snackbar: SnackbarHostState,
    onApprovals: () -> Unit,
    model: ChatViewModel = viewModel(),
) {
    val state by model.state.collectAsState()
    val list = rememberLazyListState()
    var history by remember { mutableStateOf(false) }
    var settings by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(state.messages.size, state.typing) {
        if (state.messages.isNotEmpty()) list.animateScrollToItem(state.messages.lastIndex)
    }
    Scaffold(
        modifier = Modifier.imePadding(),
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(stringResource(R.string.tab_chat))
                        Text(
                            state.status.ifBlank { if (state.connected) stringResource(R.string.chat_connected) else stringResource(R.string.chat_offline) },
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                },
                actions = {
                    IconButton(onClick = { history = true }) { Icon(Icons.Outlined.History, stringResource(R.string.chat_history)) }
                    IconButton(onClick = { model.newSession() }) { Icon(Icons.Outlined.Add, stringResource(R.string.chat_new)) }
                    IconButton(onClick = { settings = true }) { Icon(Icons.Outlined.Settings, stringResource(R.string.chat_settings)) }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface),
            )
        },
        snackbarHost = { SnackbarHost(snackbar) },
    ) { inner ->
        Column(Modifier.fillMaxSize().padding(inner).navigationBarsPadding()) {
            if (state.pendingApprovals > 0) {
                Card(
                    onClick = onApprovals,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.tertiaryContainer),
                ) {
                    Text(
                        stringResource(R.string.chat_approval_banner, state.pendingApprovals),
                        modifier = Modifier.padding(16.dp),
                        style = MaterialTheme.typography.bodyMedium,
                    )
                }
            }
            if (!state.modelConfigured) {
                Card(
                    onClick = { settings = true },
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                    colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
                ) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(stringResource(R.string.chat_setup_title), style = MaterialTheme.typography.titleSmall)
                        Text(stringResource(R.string.chat_setup_body), style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
            LazyColumn(
                state = list,
                modifier = Modifier.weight(1f).fillMaxWidth(),
                contentPadding = PaddingValues(16.dp, 8.dp, 16.dp, 16.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                if (state.empty) {
                    item { EmptyChat(onPrompt = { model.send(it) }) }
                }
                items(state.messages, key = { it.id }) { message ->
                    MessageBubble(message)
                }
                if (state.typing) {
                    item {
                        Text(stringResource(R.string.chat_typing), color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyMedium)
                    }
                }
                state.error?.let { err ->
                    item { Text(err, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
                }
            }
            Composer(
                value = state.drafting,
                onValue = model::draft,
                onSend = { model.send() },
                onStop = { model.stopTyping() },
                typing = state.typing,
                enabled = state.runtimeReady,
            )
        }
    }
    if (history) {
        ModalBottomSheet(onDismissRequest = { history = false }, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
            Text(stringResource(R.string.chat_history), style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp))
            if (state.sessions.isEmpty()) {
                Text(stringResource(R.string.chat_history_empty), modifier = Modifier.padding(24.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
            } else {
                state.sessions.forEach { session ->
                    TextButton(
                        onClick = { model.openSession(session.id); history = false },
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp),
                    ) {
                        Column(Modifier.fillMaxWidth()) {
                            Text(session.title.ifBlank { stringResource(R.string.chat_new) }, style = MaterialTheme.typography.bodyLarge)
                        }
                    }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
    if (settings) {
        SettingsSheet(
            initial = model.currentSettings(),
            onDismiss = { settings = false },
            onSave = { next ->
                model.saveModel(next) { err ->
                    settings = false
                    if (err != null) scope.launch { snackbar.showSnackbar(err) }
                }
            },
        )
    }
}

@Composable
private fun EmptyChat(onPrompt: (String) -> Unit) {
    Column(Modifier.fillMaxWidth().padding(top = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(stringResource(R.string.chat_empty_title), style = MaterialTheme.typography.headlineSmall)
        Text(stringResource(R.string.chat_empty_body), color = MaterialTheme.colorScheme.onSurfaceVariant)
        val chips = listOf(
            stringResource(R.string.chat_chip_devices),
            stringResource(R.string.chat_chip_build),
            stringResource(R.string.chat_chip_phone),
        )
        LazyRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            items(chips) { label -> SuggestionChip(onClick = { onPrompt(label) }, label = { Text(label) }) }
        }
    }
}

@Composable
private fun MessageBubble(message: ChatMessage) {
    val mine = message.role == ChatMessage.Role.User
    val context = LocalContext.current
    Row(Modifier.fillMaxWidth(), horizontalArrangement = if (mine) Arrangement.End else Arrangement.Start) {
        Card(
            modifier = Modifier.widthIn(max = 560.dp).fillMaxWidth(if (mine) 0.86f else 0.94f),
            colors = CardDefaults.cardColors(
                containerColor = if (mine) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerLow,
            ),
            shape = RoundedCornerShape(18.dp),
        ) {
            Column(Modifier.padding(14.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (!mine && message.kind == PicoProtocol.KIND_THOUGHT) {
                    Text(stringResource(R.string.chat_thought), style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                message.toolCalls.forEach { call ->
                    AssistChip(onClick = {}, label = { Text(toolLabel(call.name, call.status, call.detail)) })
                }
                if (message.content.isNotBlank()) {
                    if (mine) Text(message.content, style = MaterialTheme.typography.bodyLarge)
                    else MarkdownText(message.content)
                }
                if (!mine && message.content.isNotBlank()) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        message.modelName?.let { Text(it, style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
                        Spacer(Modifier.weight(1f))
                        IconButton(onClick = {
                            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                            clipboard.setPrimaryClip(ClipData.newPlainText("cranibox", message.content))
                        }, modifier = Modifier.size(32.dp)) {
                            Icon(Icons.Outlined.ContentCopy, stringResource(R.string.chat_copy), modifier = Modifier.size(16.dp))
                        }
                    }
                }
            }
        }
    }
}

private fun toolLabel(name: String, status: String?, detail: String?): String {
    val pretty = when {
        name.contains("cranibox") && name.contains("device") -> "正在查看设备"
        name.contains("computer") || name.contains("screenshot") || name.contains("computer_use") -> "正在操作屏幕"
        name.contains("exec") || name.contains("command") || name.contains("shell") -> "正在执行命令"
        name.contains("push") || name.contains("pull") || name.contains("copy") -> "正在传输文件"
        name.contains("task") || name.contains("job") -> "正在处理任务"
        else -> "正在调用 $name"
    }
    return listOfNotNull(pretty, status?.takeIf { it.isNotBlank() && it != "running" }, detail?.take(24)).joinToString(" · ")
}

@Composable
private fun Composer(
    value: String,
    onValue: (String) -> Unit,
    onSend: () -> Unit,
    onStop: () -> Unit,
    typing: Boolean,
    enabled: Boolean,
) {
    Row(
        Modifier.fillMaxWidth().padding(12.dp),
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        OutlinedTextField(
            value = value,
            onValueChange = onValue,
            modifier = Modifier.weight(1f),
            placeholder = { Text(stringResource(R.string.chat_placeholder)) },
            enabled = enabled,
            maxLines = 6,
            shape = RoundedCornerShape(20.dp),
        )
        if (typing) {
            FilledIconButton(onClick = onStop) { Icon(Icons.Outlined.Stop, stringResource(R.string.chat_stop)) }
        } else {
            FilledIconButton(onClick = onSend, enabled = enabled && value.isNotBlank()) {
                Icon(Icons.AutoMirrored.Outlined.Send, stringResource(R.string.chat_send))
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SettingsSheet(
    initial: PicoClawConfig.ModelSettings,
    onDismiss: () -> Unit,
    onSave: (PicoClawConfig.ModelSettings) -> Unit,
) {
    var useGateway by remember { mutableStateOf(initial.useLocalGateway) }
    var base by remember { mutableStateOf(initial.customApiBase) }
    var key by remember { mutableStateOf(initial.customApiKey) }
    var model by remember { mutableStateOf(initial.customModel) }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.padding(horizontal = 24.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(stringResource(R.string.chat_settings), style = MaterialTheme.typography.titleLarge)
            Text(stringResource(R.string.chat_settings_intro), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.chat_use_gateway), modifier = Modifier.weight(1f))
                Switch(checked = useGateway, onCheckedChange = { useGateway = it })
            }
            OutlinedTextField(value = base, onValueChange = { base = it }, label = { Text(stringResource(R.string.chat_api_base)) }, modifier = Modifier.fillMaxWidth(), placeholder = { Text("https://api.openai.com/v1") }, singleLine = true)
            OutlinedTextField(value = key, onValueChange = { key = it }, label = { Text(stringResource(R.string.chat_api_key)) }, modifier = Modifier.fillMaxWidth(), visualTransformation = PasswordVisualTransformation(), singleLine = true)
            OutlinedTextField(value = model, onValueChange = { model = it }, label = { Text(stringResource(R.string.chat_model_id)) }, modifier = Modifier.fillMaxWidth(), singleLine = true)
            TextButton(onClick = { onSave(PicoClawConfig.ModelSettings(useGateway, base, key, model)) }, modifier = Modifier.align(Alignment.End)) {
                Text(stringResource(R.string.save))
            }
            Spacer(Modifier.height(16.dp))
        }
    }
}
