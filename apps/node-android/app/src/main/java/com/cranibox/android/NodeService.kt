package com.cranibox.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.RemoteInput
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import com.cranibox.android.notify.ApprovalNotifier
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** User-started foreground service. Android never starts phone control silently at boot. */
class NodeService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var runtime: NodeRuntime? = null
    private var running: Job? = null
    private val runtimeLock = Mutex()
    @Volatile private var stopping = false
    private var pairingPrompt = false
    private val app get() = node
    private val notifications get() = getSystemService(NotificationManager::class.java)

    override fun onCreate() {
        super.onCreate()
        app.acceptHelpers = true
        notifications.createNotificationChannel(NotificationChannel(CHANNEL, "手机节点", NotificationManager.IMPORTANCE_LOW))
        if (Build.VERSION.SDK_INT >= 34) startForeground(ID, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        else startForeground(ID, notification())
        scope.launch { app.status.collectLatest { update() } }
        scope.launch { app.adb.state.collectLatest { update() } }
        scope.launch { app.phone.connected.collectLatest { connected ->
            runCatching { app.adb.writeUsbScript() }
            if (!connected && app.adb.state.value.startsWith("手机控制已连接")) app.adb.state.value = "助手已断开，可点击重新连接"
            update()
        } }
        app.repository.start(scope, ApprovalNotifier(this))
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            RESTART -> {
                scope.launch { startRuntime(restart = true) }
                return START_STICKY
            }
            STOP -> {
                stopping = true
                app.acceptHelpers = false
                scope.launch {
                    runtimeLock.withLock {
                        // Even a full disk must not prevent the user from stopping the processes.
                        runCatching { app.bridge.emergencyStop("在手机上停止节点") }
                        app.phone.disconnect(true)
                        running?.cancelAndJoin()
                        app.status.value = "节点已停止"
                        stopForeground(STOP_FOREGROUND_REMOVE)
                        stopSelf()
                    }
                }
                return START_NOT_STICKY
            }
            EMERGENCY -> scope.launch {
                // Stop input immediately, then revoke the node's remote control lease too.
                app.bridge.emergencyStop("在手机上停止 Agent")
                runCatching { runtimeLock.withLock { runtime?.setComputerPaused(true) } }
                    .onFailure { android.util.Log.e("CraniboxNode", "Cannot persist node control stop", it) }
                update()
            }
            RESUME -> scope.launch {
                try {
                    runtimeLock.withLock {
                        checkNotNull(runtime) { "node is not running" }.setComputerPaused(false)
                    }
                } catch (error: Exception) {
                    // Never show a successful resume while the node still refuses control.
                    app.bridge.emergencyStop("恢复失败，请重试或查看节点日志")
                    android.util.Log.e("CraniboxNode", "Cannot resume node control", error)
                }
                update()
            }
            SHOW_PAIR -> { pairingPrompt = true; update() }
            PAIR -> {
                val code = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(CODE)?.toString()
                    ?: intent.getStringExtra(CODE) ?: ""
                val port = intent.getIntExtra(PORT, 0).takeIf { it in 1..65535 }
                scope.launch { app.adb.pair(code.trim(), port); pairingPrompt = false; update() }
            }
            CONNECT -> scope.launch { app.adb.connect(intent.getIntExtra(PORT, 0).takeIf { it in 1..65535 }) }
            FORGET -> scope.launch { app.adb.forget(); update() }
        }
        scope.launch { startRuntime() }
        return START_STICKY
    }

    private suspend fun startRuntime(restart: Boolean = false) = runtimeLock.withLock {
        if (stopping) return@withLock
        if (restart) running?.cancelAndJoin()
        if (running?.isActive != true) {
            val next = NodeRuntime(app)
            runtime = next
            running = scope.launch {
                try { next.run() }
                catch (e: CancellationException) { throw e }
                catch (e: Exception) { app.status.value = e.message?.take(180) ?: "节点启动失败"; update() }
            }
        }
    }

    private fun update() { notifications.notify(ID, notification()) }

    private fun notification(): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val text = if (app.bridge.paused) "Agent 已暂停 · ${app.status.value}" else "${app.status.value} · ${app.adb.state.value}"
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification).setContentTitle("Cranibox 手机节点")
            .setContentText(text).setStyle(Notification.BigTextStyle().bigText(text))
            .setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
            .addAction(Notification.Action.Builder(null, "停止 Agent", pending(EMERGENCY, 1)).build())
            .apply {
                if (pairingPrompt) addAction(Notification.Action.Builder(null, "输入配对码", pending(PAIR, 2, mutable = true))
                    .addRemoteInput(RemoteInput.Builder(CODE).setLabel("无线调试的六位配对码").build()).build())
                else addAction(Notification.Action.Builder(null, "停止节点", pending(STOP, 3)).build())
            }.build()
    }

    private fun pending(action: String, request: Int, mutable: Boolean = false): PendingIntent = PendingIntent.getService(
        this, request, Intent(this, NodeService::class.java).setAction(action),
        PendingIntent.FLAG_UPDATE_CURRENT or if (mutable) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE,
    )

    override fun onDestroy() {
        scope.cancel()
        // Binder and child shutdown may block; keep them off Android's main thread.
        Thread {
            runtime?.close()
            if (!stopping) app.phone.disconnect(true)
        }.start()
        app.loginUrl.value = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        private const val CHANNEL = "cranibox-node"
        private const val ID = 1
        const val STOP = "com.cranibox.node.STOP"
        const val RESTART = "com.cranibox.node.RESTART"
        const val EMERGENCY = "com.cranibox.node.EMERGENCY"
        const val RESUME = "com.cranibox.node.RESUME"
        const val PAIR = "com.cranibox.node.PAIR"
        const val SHOW_PAIR = "com.cranibox.node.SHOW_PAIR"
        const val CONNECT = "com.cranibox.node.CONNECT"
        const val FORGET = "com.cranibox.node.FORGET"
        const val CODE = "code"
        const val PORT = "port"
    }
}
