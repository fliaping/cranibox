package com.cranibox.android.api

object Copy {
    val capabilities = listOf(
        "messages", "terminals", "exec", "projects", "desktop", "computer",
        "browser", "update", "spaces", "activity", "llm", "vault",
    )

    val groups = listOf(
        CapGroup("消息与任务", "messages"),
        CapGroup("终端与命令", "terminals", "exec", "projects"),
        CapGroup("屏幕", "desktop", "computer"),
        CapGroup("浏览器", "browser"),
        CapGroup("远程更新", "update"),
        CapGroup("虚拟空间", "spaces"),
        CapGroup("活动与同步", "activity"),
        CapGroup("模型与账号", "llm", "vault"),
    )

    fun defaultModes(): Map<String, String> =
        capabilities.associateWith { if (it == "messages") "allow" else "off" }

    fun approvalTitle(kind: String): String = when (kind) {
        "pairing" -> "核对新设备"
        "message" -> "一条设备消息等待确认"
        "capability" -> "设备请求新的权限"
        "agent_tool" -> "Agent 请求使用工具"
        "computer_use" -> "允许操作你的屏幕？"
        "model_write" -> "确认修改模型配置"
        else -> "需要你确认"
    }

    fun capabilityName(id: String): String = when (id) {
        "messages" -> "消息"
        "terminals" -> "终端"
        "exec" -> "命令"
        "projects" -> "项目"
        "desktop" -> "远程桌面"
        "computer" -> "Computer Use"
        "browser" -> "浏览器"
        "update" -> "远程更新"
        "spaces" -> "虚拟空间"
        "activity" -> "活动"
        "llm" -> "模型"
        "vault" -> "账号凭据"
        else -> id
    }

    fun operationName(kind: String?): String {
        if (kind.isNullOrBlank()) return ""
        return when (kind) {
            "exec.run" -> "运行命令"
            "exec.finished" -> "命令结束"
            "exec.status" -> "查看命令结果"
            "exec.cancel" -> "停止命令"
            "files.write" -> "写入文件"
            "files.read" -> "读取文件"
            "files.list" -> "查看文件夹内容"
            "terminal.list" -> "查看终端列表"
            "terminal.create" -> "新建终端"
            "terminal.rename" -> "重命名终端"
            "terminal.end" -> "结束终端"
            "terminal.remove" -> "删除终端"
            "terminal.attach" -> "连接终端"
            "desktop.view" -> "查看并控制屏幕"
            "projects.list" -> "查看项目"
            "projects.sync" -> "同步项目"
            "update.check" -> "检查更新"
            "update.install" -> "安装更新"
            "messages.send" -> "发送消息"
            "messages.read" -> "读取消息"
            "tasks.send" -> "发送任务"
            "tasks.read" -> "查看任务"
            "llm.use" -> "使用模型"
            "vault.pull" -> "读取账号凭据"
            "vault.push" -> "写入账号凭据"
            else -> when (kind.substringBefore('.')) {
                "computer" -> "控制电脑"
                "browser" -> "控制浏览器"
                "agent" -> "运行智能体"
                "vault" -> "使用账号凭据"
                "projects" -> "修改项目"
                "terminal" -> "管理终端"
                else -> kind
            }
        }
    }

    fun kindLabel(kind: String?): String = when (kind) {
        "phone" -> "手机"
        "desktop" -> "电脑"
        "laptop" -> "笔记本"
        "server" -> "服务器"
        "space" -> "虚拟空间"
        else -> kind ?: "设备"
    }

    fun taskState(state: String): String = when (state) {
        "running", "active", "in_progress" -> "进行中"
        "queued", "pending" -> "等待中"
        "done", "finished", "succeeded" -> "已完成"
        "failed", "error" -> "失败"
        "cancelled", "canceled" -> "已取消"
        else -> state
    }

    data class CapGroup(val title: String, val caps: List<String>) {
        constructor(title: String, vararg caps: String) : this(title, caps.toList())
    }
}
