package com.cranibox.android.ui.devices

import android.app.Application
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import kotlinx.coroutines.launch
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewmodel.compose.viewModel
import com.cranibox.android.NodeApp
import com.cranibox.android.R
import com.cranibox.android.api.Copy
import com.cranibox.android.api.Device

class DevicesViewModel(application: Application) : AndroidViewModel(application) {
    val repo = (application as NodeApp).repository
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DevicesScreen(onDevice: (String) -> Unit, onScan: () -> Unit, model: DevicesViewModel = viewModel()) {
    val devices by model.repo.devices.collectAsState()
    val ready by model.repo.signedIn.collectAsState()
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(stringResource(R.string.tab_devices)) },
            actions = {
                IconButton(onClick = onScan) {
                    Icon(Icons.Outlined.QrCodeScanner, contentDescription = stringResource(R.string.scan_pair))
                }
            },
        )
        if (!ready) {
            Text(stringResource(R.string.node_not_ready), modifier = Modifier.padding(20.dp), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    runCatching { model.repo.refresh() }
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize(),
        ) {
            LazyColumn(
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(16.dp, 8.dp, 16.dp, 24.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (devices.isEmpty()) {
                    item {
                        Text(
                            stringResource(R.string.devices_empty),
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(8.dp),
                        )
                    }
                }
                items(devices, key = { it.nodeId }) { device ->
                    DeviceCard(device) { onDevice(device.nodeId) }
                }
            }
        }
    }
}

@Composable
fun DeviceCard(device: Device, onClick: () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
    ) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier.size(12.dp).clip(CircleShape).background(
                    if (device.online) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                ),
            )
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(device.name.ifBlank { device.nodeId }, style = MaterialTheme.typography.titleMedium)
                val bits = buildList {
                    add(if (device.online) "在线" else "离线")
                    if (device.isThis) add("本机")
                    device.osVersion?.let { add(it) }
                    if (device.kind.isNotBlank()) add(Copy.kindLabel(device.kind))
                    device.model?.let { add(it) }
                }
                Text(bits.joinToString(" · "), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
