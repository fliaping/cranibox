package com.cranibox.android.api

import com.cranibox.android.NodeApp
import com.cranibox.android.notify.ApprovalNotifier
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

class NodeRepository(private val app: NodeApp) {
    val client = NodeClient()
    val devices = MutableStateFlow<List<Device>>(emptyList())
    val approvals = MutableStateFlow<List<ApprovalCard>>(emptyList())
    val tasks = MutableStateFlow<List<TaskSummary>>(emptyList())
    val access = MutableStateFlow<AccessView?>(null)
    val remote = MutableStateFlow<RemoteView?>(null)
    val error = MutableStateFlow<String?>(null)
    val signedIn get() = client.ready
    private var poll: Job? = null

    fun start(scope: CoroutineScope, notifier: ApprovalNotifier) {
        scope.launch {
            app.loginUrl.collectLatest { url ->
                poll?.cancel(); poll = null
                if (url.isNullOrBlank()) {
                    client.disconnect()
                    devices.value = emptyList()
                    approvals.value = emptyList()
                    tasks.value = emptyList()
                    access.value = null
                    remote.value = null
                    notifier.reset()
                    return@collectLatest
                }
                try {
                    client.signIn(url)
                    error.value = null
                    refresh()
                    poll = scope.launch {
                        while (true) {
                            delay(4_000)
                            runCatching { refresh() }.onFailure { failure ->
                                error.value = failure.message
                                if (failure is ApiException && failure.status == 401) {
                                    runCatching { client.signIn(url); refresh() }
                                }
                            }
                        }
                    }
                } catch (e: Exception) {
                    error.value = e.message
                    notifier.reset()
                }
            }
        }
        scope.launch {
            combine(approvals, access, remote) { cards, accessView, remoteView ->
                Triple(cards, accessView, remoteView)
            }.collect { (cards, accessView, remoteView) ->
                val local = accessView?.pending.orEmpty()
                val remoteItems = remoteView?.devices.orEmpty().flatMap { device ->
                    device.requests.map { it.request }
                }
                val ids = cards.map { "inbox:" + it.id } +
                    local.map { "access:" + it.id } +
                    remoteItems.map { "remote:" + it.id }
                val titles = cards.map { Copy.approvalTitle(it.kind) } +
                    local.map { it.operation?.summary?.ifBlank { null } ?: Copy.capabilityName(it.capability) } +
                    remoteItems.map { it.operation?.summary?.ifBlank { null } ?: Copy.capabilityName(it.capability) }
                notifier.publish(ids, titles)
            }
        }
    }

    suspend fun refresh() {
        if (!client.ready.value) return
        val nextDevices = runCatching { client.devices() }
        val nextApprovals = runCatching { client.approvals() }
        val nextTasks = runCatching { client.tasks() }
        val nextAccess = runCatching { client.access() }
        val nextRemote = runCatching { client.remote() }
        nextDevices.onSuccess { devices.value = it }
        nextApprovals.onSuccess { approvals.value = it }
        nextTasks.onSuccess { tasks.value = it }
        nextAccess.onSuccess { access.value = it }
        nextRemote.onSuccess { remote.value = it }
        val failure = listOf(nextDevices, nextApprovals, nextAccess)
            .firstOrNull { it.isFailure }?.exceptionOrNull()
        error.value = failure?.message
        if (failure != null) throw failure
    }
}
