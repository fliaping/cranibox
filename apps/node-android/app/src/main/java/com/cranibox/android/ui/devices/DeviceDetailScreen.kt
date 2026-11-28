package com.cranibox.android.ui.devices

import android.app.Application
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarHostState
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
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import com.cranibox.android.NodeApp
import com.cranibox.android.R
import com.cranibox.android.api.Copy
import kotlinx.coroutines.launch

class DeviceDetailViewModel(application: Application) : AndroidViewModel(application) {
    val repo = (application as NodeApp).repository
    fun rename(node: String, name: String, onDone: (String?) -> Unit) =
        viewModelScope.launch { onDone(runCatching { repo.client.renameDevice(node, name); repo.refresh() }.exceptionOrNull()?.message) }
    fun setModes(node: String, modes: Map<String, String>, onDone: (String?) -> Unit) =
        viewModelScope.launch { onDone(runCatching { repo.client.setAccessModes(node, modes); repo.refresh() }.exceptionOrNull()?.message) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DeviceDetailScreen(
    nodeId: String,
    onBack: () -> Unit,
    onSession: (String) -> Unit,
    snackbar: SnackbarHostState,
    model: DeviceDetailViewModel = viewModel(),
) {
    val devices by model.repo.devices.collectAsState()
    val access by model.repo.access.collectAsState()
    val device = devices.find { it.nodeId == nodeId }
    val modes = access?.peers?.find { it.node == nodeId }?.capabilities ?: Copy.defaultModes()
    var renaming by remember { mutableStateOf(false) }
    var draftName by remember { mutableStateOf(device?.name.orEmpty()) }
    val scope = rememberCoroutineScope()
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(device?.name?.ifBlank { nodeId } ?: nodeId) },
            navigationIcon = {
                IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = "返回") }
            },
            actions = {
                IconButton(onClick = { draftName = device?.name.orEmpty(); renaming = true }) {
                    Icon(Icons.Outlined.Edit, contentDescription = stringResource(R.string.rename_device))
                }
            },
        )
        Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (device == null) {
                Text("正在加载设备…", color = MaterialTheme.colorScheme.onSurfaceVariant)
                return@Column
            }
            Text(
                buildList {
                    add(if (device.online) stringResource(R.string.online) else stringResource(R.string.offline))
                    if (device.isThis) add(stringResource(R.string.this_device))
                    device.osVersion?.let(::add)
                    if (device.kind.isNotBlank()) add(Copy.kindLabel(device.kind))
                    device.model?.let(::add)
                }.joinToString(" · "),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (!device.isThis) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    val terminalOk = device.caps["terminals"] != false
                    val desktopOk = device.caps["desktop"] != false
                    OutlinedButton(onClick = { onSession("terminal") }, enabled = terminalOk && device.online) {
                        Text(stringResource(R.string.open_terminal))
                    }
                    Button(onClick = { onSession("desktop") }, enabled = desktopOk && device.online) {
                        Text(stringResource(R.string.open_desktop))
                    }
                }
            }
            Text(stringResource(R.string.capability_toggles), style = MaterialTheme.typography.titleMedium)
            Copy.groups.forEach { group ->
                val current = group.caps.map { modes[it] ?: "off" }.distinct().singleOrNull()
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(group.title, style = MaterialTheme.typography.bodyLarge)
                    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                        listOf("off" to stringResource(R.string.mode_off), "ask" to stringResource(R.string.mode_ask), "allow" to stringResource(R.string.mode_allow)).forEachIndexed { index, (mode, label) ->
                            SegmentedButton(
                                selected = current == mode,
                                onClick = {
                                    val next = modes.toMutableMap()
                                    group.caps.forEach { next[it] = mode }
                                    model.setModes(nodeId, next) { err -> err?.let { scope.launch { snackbar.showSnackbar(it) } } }
                                },
                                shape = SegmentedButtonDefaults.itemShape(index, 3),
                            ) { Text(label) }
                        }
                    }
                }
            }
        }
    }
    if (renaming) {
        AlertDialog(
            onDismissRequest = { renaming = false },
            title = { Text(stringResource(R.string.rename_device)) },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(stringResource(R.string.rename_hint), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    OutlinedTextField(value = draftName, onValueChange = { if (it.length <= 64) draftName = it }, singleLine = true, modifier = Modifier.fillMaxWidth())
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    model.rename(nodeId, draftName.trim()) { err ->
                        if (err == null) renaming = false
                        err?.let { scope.launch { snackbar.showSnackbar(it) } }
                    }
                }) { Text(stringResource(R.string.save)) }
            },
            dismissButton = { TextButton(onClick = { renaming = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
