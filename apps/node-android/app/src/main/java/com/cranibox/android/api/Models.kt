package com.cranibox.android.api

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonElement

@Serializable
data class Device(
    @SerialName("node_id") val nodeId: String,
    val name: String,
    val kind: String = "",
    val online: Boolean = false,
    val path: String = "unknown",
    @SerialName("rtt_ms") val rttMs: Long? = null,
    val via: String? = null,
    val badges: List<String> = emptyList(),
    val guest: Boolean = false,
    val relation: String? = null,
    @SerialName("same_identity") val sameIdentity: Boolean? = null,
    val caps: Map<String, Boolean> = emptyMap(),
    @SerialName("os_version") val osVersion: String? = null,
    val model: String? = null,
    @SerialName("last_seen") val lastSeen: Long? = null,
) {
    val isThis: Boolean get() = "this" in badges
}

@Serializable
data class ApprovalCard(
    val id: String,
    val kind: String,
    @SerialName("from_node") val fromNode: String? = null,
    @SerialName("on_node") val onNode: String = "",
    val agent: String? = null,
    @SerialName("title_key") val titleKey: String = "",
    @SerialName("title_args") val titleArgs: Map<String, String> = emptyMap(),
    val detail: String? = null,
    val risk: String = "medium",
    @SerialName("created_at") val createdAt: Long = 0,
    @SerialName("expires_at") val expiresAt: Long? = null,
    val actions: List<String> = emptyList(),
)

@Serializable
data class TaskSummary(
    val id: String,
    val title: String = "",
    val direction: String = "",
    val state: String = "",
    val progress: String? = null,
    val result: String? = null,
    val questions: List<TaskQuestion>? = null,
    @SerialName("updated_at") val updatedAt: Long = 0,
)

@Serializable
data class TaskQuestion(
    val id: String,
    val text: String = "",
    val answer: String? = null,
)

@Serializable
data class AccessView(
    @SerialName("approvals_disabled") val approvalsDisabled: Boolean = false,
    val peers: List<AccessPeer> = emptyList(),
    val pending: List<AccessRequest> = emptyList(),
    val approvals: List<AccessApproval> = emptyList(),
    val sessions: List<AccessSession> = emptyList(),
)

@Serializable
data class AccessPeer(
    val node: String,
    val name: String = "",
    val capabilities: Map<String, String> = emptyMap(),
)

@Serializable
data class AccessRequest(
    val id: String,
    val from: String = "",
    val name: String = "",
    val capability: String = "",
    @SerialName("created_at") val createdAt: Long = 0,
    @SerialName("expires_at") val expiresAt: Long = 0,
    val state: String = "",
    val path: JsonElement? = null,
    val operation: AccessOperation? = null,
)

@Serializable
data class AccessOperation(
    val kind: String = "",
    val risk: String = "",
    val summary: String = "",
    val confirm: Boolean = false,
)

@Serializable
data class AccessApproval(
    val id: String,
    val from: String = "",
    val capability: String = "",
    val scope: String? = null,
    @SerialName("expires_at") val expiresAt: Long? = null,
)

@Serializable
data class AccessSession(
    val id: String,
    val from: String = "",
    val name: String = "",
    val capability: String = "",
)

@Serializable
data class RemoteView(
    val devices: List<RemoteDevice> = emptyList(),
    val approver: Approver? = null,
)

@Serializable
data class RemoteDevice(
    val node: String,
    val name: String = "",
    val online: Boolean = false,
    val error: String? = null,
    val requests: List<RemotePending> = emptyList(),
)

@Serializable
data class RemotePending(
    val request: AccessRequest,
    @SerialName("op_hash") val opHash: String = "",
    @SerialName("once_only") val onceOnly: Boolean = false,
    @SerialName("challenge_expires_at") val challengeExpiresAt: Long = 0,
)

@Serializable
data class Approver(
    val recovery: Boolean = false,
    @SerialName("recovery_kind") val recoveryKind: String? = null,
    @SerialName("remote_enabled") val remoteEnabled: Boolean = false,
    @SerialName("owner_device") val ownerDevice: Boolean = false,
    val attempts: Int = 0,
)

@Serializable
data class PairAccepted(
    @SerialName("flow_id") val flowId: String,
    val sas: String = "",
    val purpose: String? = null,
)

class ApiException(
    message: String,
    val status: Int = 0,
    val code: String = "",
) : RuntimeException(message)
