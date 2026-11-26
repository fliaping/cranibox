package com.cranibox.android.ui.home

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import com.cranibox.android.R
import com.cranibox.android.api.Copy
import com.cranibox.android.api.RemotePending

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ApprovalSheet(
    item: PendingItem,
    recoveryRequired: Boolean,
    recoveryKind: String?,
    onDismiss: () -> Unit,
    onInbox: (String, String) -> Unit,
    onAccess: (String, Boolean, String?) -> Unit,
    onRemote: (String, RemotePending, Boolean, String?, String) -> Unit,
) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var code by remember { mutableStateOf("") }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = sheet) {
        Column(
            Modifier.fillMaxWidth().padding(horizontal = 24.dp).padding(bottom = 32.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            val title = when (item) {
                is PendingItem.Inbox -> Copy.approvalTitle(item.card.kind)
                is PendingItem.Local -> heading(item.request.name, item.request.capability, item.request.operation?.kind, item.request.operation?.summary)
                is PendingItem.Remote -> heading(item.deviceName, item.pending.request.capability, item.pending.request.operation?.kind, item.pending.request.operation?.summary)
            }
            Text(title, style = MaterialTheme.typography.headlineSmall)
            when (item) {
                is PendingItem.Inbox -> {
                    item.card.titleArgs.forEach { (key, value) ->
                        if (key != "purpose") {
                            Text(if (key == "code") value else value, style = if (key == "code") MaterialTheme.typography.headlineMedium else MaterialTheme.typography.bodyLarge)
                        }
                    }
                    item.card.detail?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
                }
                is PendingItem.Local -> item.request.operation?.summary?.let {
                    Text(it, style = MaterialTheme.typography.bodyLarge, fontFamily = FontFamily.Monospace)
                }
                is PendingItem.Remote -> item.pending.request.operation?.summary?.let {
                    Text(it, style = MaterialTheme.typography.bodyLarge, fontFamily = FontFamily.Monospace)
                }
            }
            if (recoveryRequired) {
                Text(stringResource(R.string.recovery_hint), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                OutlinedTextField(
                    value = code,
                    onValueChange = { code = it },
                    modifier = Modifier.fillMaxWidth(),
                    label = { Text(stringResource(R.string.recovery_label)) },
                    visualTransformation = PasswordVisualTransformation(),
                    singleLine = true,
                )
            }
            val canApprove = !recoveryRequired || code.isNotBlank()
            when (item) {
                is PendingItem.Inbox -> {
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = { onInbox(item.card.id, "deny") }) { Text(stringResource(R.string.deny)) }
                        item.card.actions.filter { it != "deny" }.forEach { action ->
                            val label = when {
                                action == "allow_always" -> stringResource(R.string.allow_always)
                                item.card.kind == "pairing" -> stringResource(R.string.codes_match)
                                else -> stringResource(R.string.allow_once)
                            }
                            Button(onClick = { onInbox(item.card.id, action) }) { Text(label) }
                        }
                    }
                }
                is PendingItem.Local, is PendingItem.Remote -> {
                    val onceOnly = when (item) {
                        is PendingItem.Local -> item.request.operation?.confirm == true
                        is PendingItem.Remote -> item.pending.onceOnly || item.pending.request.operation?.confirm == true
                        else -> true
                    }
                    val scopes = if (onceOnly) listOf("session" to stringResource(R.string.allow_once))
                    else listOf(
                        "session" to stringResource(R.string.allow_once),
                        "1h" to stringResource(R.string.allow_1h),
                        "today" to stringResource(R.string.allow_today),
                        "7d" to stringResource(R.string.allow_7d),
                    )
                    OutlinedButton(onClick = {
                        when (item) {
                            is PendingItem.Local -> onAccess(item.request.id, false, null)
                            is PendingItem.Remote -> onRemote(item.node, item.pending, false, null, code)
                            else -> Unit
                        }
                    }, enabled = canApprove, modifier = Modifier.fillMaxWidth()) { Text(stringResource(R.string.deny)) }
                    scopes.forEach { (scope, label) ->
                        Button(onClick = {
                            when (item) {
                                is PendingItem.Local -> onAccess(item.request.id, true, scope)
                                is PendingItem.Remote -> onRemote(item.node, item.pending, true, scope, code)
                                else -> Unit
                            }
                        }, enabled = canApprove, modifier = Modifier.fillMaxWidth()) { Text(label) }
                    }
                }
            }
        }
    }
}

private fun heading(name: String, capability: String, kind: String?, summary: String?): String {
    val who = name.ifBlank { "一台设备" }
    val action = summary?.takeIf { it.isNotBlank() } ?: Copy.operationName(kind).ifBlank { Copy.capabilityName(capability) }
    return "$who 请求$action"
}
