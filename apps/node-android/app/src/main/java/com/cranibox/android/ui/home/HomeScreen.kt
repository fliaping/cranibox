package com.cranibox.android.ui.home

import android.app.Application
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.compose.viewModel
import com.cranibox.android.NodeApp
import com.cranibox.android.R
import com.cranibox.android.api.AccessRequest
import com.cranibox.android.api.ApiException
import com.cranibox.android.api.ApprovalCard
import com.cranibox.android.api.Copy
import com.cranibox.android.api.RemotePending
import com.cranibox.android.api.TaskSummary
import kotlinx.coroutines.launch

class HomeViewModel(application: Application) : AndroidViewModel(application) {
    private val app = application as NodeApp
    val repo = app.repository
    fun decideInbox(id: String, decision: String, onDone: (String?) -> Unit) =
        viewModelScope.launch { onDone(runCatching { repo.client.decideApproval(id, decision); repo.refresh() }.exceptionOrNull()?.message) }
    fun decideAccess(id: String, approve: Boolean, scope: String?, onDone: (String?) -> Unit) =
        viewModelScope.launch { onDone(runCatching { repo.client.decideAccess(id, approve, scope); repo.refresh() }.exceptionOrNull()?.message) }
    fun decideRemote(node: String, pending: RemotePending, approve: Boolean, scope: String?, code: String, onDone: (String?) -> Unit) =
        viewModelScope.launch {
            val error = runCatching {
                repo.client.decideRemote(node, pending.request.id, pending.opHash, approve, scope, code)
                repo.refresh()
            }.exceptionOrNull()
            onDone(error?.let { if (it is ApiException) it.message else it.message })
        }
}

sealed class PendingItem {
    data class Inbox(val card: ApprovalCard) : PendingItem()
    data class Local(val request: AccessRequest) : PendingItem()
    data class Remote(val node: String, val deviceName: String, val pending: RemotePending) : PendingItem()
    val key: String get() = when (this) {
        is Inbox -> "inbox:${card.id}"
        is Local -> "access:${request.id}"
        is Remote -> "remote:${pending.request.id}"
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(snackbar: androidx.compose.material3.SnackbarHostState, model: HomeViewModel = viewModel()) {
    val approvals by model.repo.approvals.collectAsState()
    val tasks by model.repo.tasks.collectAsState()
    val access by model.repo.access.collectAsState()
    val remote by model.repo.remote.collectAsState()
    val ready by model.repo.signedIn.collectAsState()
    var selected by remember { mutableStateOf<PendingItem?>(null) }
    val scope = rememberCoroutineScope()
    val now = System.currentTimeMillis() / 1000
    val inbox = approvals.filter { it.expiresAt == null || it.expiresAt > now }
    val local = access?.pending.orEmpty().filter { it.expiresAt > now }
    val remotes = remote?.devices.orEmpty().flatMap { device ->
        device.requests.map { PendingItem.Remote(device.node, device.name.ifBlank { device.node }, it) }
    }
    val pending = inbox.map { PendingItem.Inbox(it) } + local.map { PendingItem.Local(it) } + remotes
    val activeTasks = tasks.filter { it.state !in setOf("done", "finished", "succeeded", "failed", "error", "cancelled", "canceled") }
    var refreshing by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(stringResource(R.string.tab_home)) },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surface),
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
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp, 8.dp, 16.dp, 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            item { Text(stringResource(R.string.approvals_section), style = MaterialTheme.typography.titleMedium) }
            if (pending.isEmpty()) {
                item {
                    EmptyCard(stringResource(R.string.approvals_empty), stringResource(R.string.approvals_empty_hint))
                }
            } else {
                items(pending, key = { it.key }) { item ->
                    PendingCard(item) { selected = item }
                }
            }
            item {
                Spacer(Modifier.height(8.dp))
                Text(stringResource(R.string.tasks_section), style = MaterialTheme.typography.titleMedium)
            }
            if (activeTasks.isEmpty()) {
                item { EmptyCard(stringResource(R.string.tasks_empty), null) }
            } else {
                items(activeTasks, key = { it.id }) { TaskCard(it) }
            }
        }
        }
    }
    selected?.let { item ->
        ApprovalSheet(
            item = item,
            recoveryRequired = item is PendingItem.Remote,
            recoveryKind = remote?.approver?.recoveryKind,
            onDismiss = { selected = null },
            onInbox = { id, decision -> model.decideInbox(id, decision) { selected = null; it?.let { msg -> scope.launch { snackbar.showSnackbar(msg) } } } },
            onAccess = { id, approve, grantScope -> model.decideAccess(id, approve, grantScope) { selected = null; it?.let { msg -> scope.launch { snackbar.showSnackbar(msg) } } } },
            onRemote = { node, pendingItem, approve, grantScope, code ->
                model.decideRemote(node, pendingItem, approve, grantScope, code) { err ->
                    if (err == null) selected = null
                    err?.let { msg -> scope.launch { snackbar.showSnackbar(msg) } }
                }
            },
        )
    }
}

@Composable
private fun PendingCard(item: PendingItem, onClick: () -> Unit) {
    val title = when (item) {
        is PendingItem.Inbox -> Copy.approvalTitle(item.card.kind)
        is PendingItem.Local -> item.request.operation?.summary?.ifBlank { null } ?: Copy.capabilityName(item.request.capability)
        is PendingItem.Remote -> item.pending.request.operation?.summary?.ifBlank { null } ?: Copy.capabilityName(item.pending.request.capability)
    }
    val subtitle = when (item) {
        is PendingItem.Inbox -> item.card.fromNode ?: item.card.agent ?: item.card.onNode
        is PendingItem.Local -> item.request.name.ifBlank { item.request.from }
        is PendingItem.Remote -> item.deviceName
    }
    Card(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onClick),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Text(subtitle, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun TaskCard(task: TaskSummary) {
    Card(Modifier.fillMaxWidth(), colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer)) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Icon(Icons.Outlined.Schedule, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
            Column(Modifier.weight(1f)) {
                Text(task.title.ifBlank { task.id }, style = MaterialTheme.typography.titleMedium)
                Text(
                    Copy.taskState(task.state) + (task.progress?.let { " · $it" } ?: ""),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun EmptyCard(title: String, hint: String?) {
    Card(Modifier.fillMaxWidth(), colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer)) {
        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(8.dp), horizontalAlignment = Alignment.Start) {
            Icon(Icons.Outlined.CheckCircle, contentDescription = null, tint = MaterialTheme.colorScheme.primary)
            Text(title, style = MaterialTheme.typography.titleMedium)
            if (hint != null) Text(hint, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
