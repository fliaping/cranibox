package com.cranibox.android

import android.content.Context
import android.content.pm.PackageManager
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.ssl.SSLSockets
import android.net.wifi.WifiManager
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import com.cranibox.android.core.adb.*
import com.cranibox.android.core.helper.HelperHandoff
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.io.File
import java.security.KeyStore
import java.net.NetworkInterface
import java.net.Inet4Address
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

class PhoneSetup(private val app: NodeApp) {
    private val mutex = Mutex()
    private val prefs = app.getSharedPreferences("adb", Context.MODE_PRIVATE)
    private val identity = AdbIdentitySource(
        WrappedKeyFile(File(app.filesDir, "adb-key.enc"), ::wrappingKey), { "Cranibox@android" },
    )
    val state = MutableStateFlow("手机控制未连接")
    val busy = MutableStateFlow(false)
    private val finder = AndroidAdbFinder(app)

    suspend fun pair(code: String, port: Int?) = operation {
        require(code.matches(Regex("[0-9]{6}"))) { "请输入六位配对码" }
        state.value = "正在查找配对窗口…"
        val endpoints = port?.let { p -> localAdbHosts().map { AdbEndpoint(it, p) } }
            ?: finder.findEndpoints(AdbServiceTypes.PAIRING, 4_000, localAdbHosts())
        check(endpoints.isNotEmpty()) { "没有找到本机配对窗口，请保持系统配对弹窗打开，或手动填写配对端口" }
        val tls = tls(identity.forPairing())
        var paired = false
        var failure: Exception? = null
        for (candidate in endpoints) {
            try {
                state.value = "正在配对…"
                AdbPairing(tls).pair(candidate.host, candidate.port, code)
                paired = true
                break
            } catch (e: kotlinx.coroutines.CancellationException) { throw e }
            catch (e: Exception) { failure = e }
        }
        if (!paired) throw failure ?: IllegalStateException("配对失败")
        kotlinx.coroutines.currentCoroutineContext().ensureActive()
        prefs.edit().putBoolean("paired", true).apply()
        try { connectHelper(null) }
        catch (e: kotlinx.coroutines.CancellationException) { throw e }
        catch (e: Exception) {
            android.util.Log.e("CraniboxAdb", "Paired, but helper connection failed", e)
            state.value = "配对已成功，启动未完成：${errorText(e)}。请在下方填写连接端口重试，无需重新配对"
        }
    }

    suspend fun connect(port: Int?) = operation { connectHelper(port) }

    suspend fun forget() = operation {
        app.phone.disconnect(true)
        app.nonces.clear()
        identity.forget()
        prefs.edit().clear().apply()
        writeUsbScript()
        state.value = "已清除本机配对；也可在系统无线调试中移除 Cranibox"
    }

    private suspend fun connectHelper(manual: Int?) {
        if (app.phone.connected.value) { state.value = "手机控制已连接"; return }
        state.value = "正在连接无线调试…"
        val tls = tls(identity.forConnect())
        val port = AdbConnectPort(
            property = { null }, finder = finder,
            cache = object : PortCache {
                override fun load() = prefs.getInt("port", 0).takeIf { it in 1..65535 }
                override fun save(port: Int) { prefs.edit().putInt("port", port).apply() }
            },
            scan = LocalPortScan(NioPortScanner(), ::localAdbHosts),
            manual = { manual }, scanning = { if (it) state.value = "正在本机查找无线调试端口…" },
            localHosts = ::localAdbHosts,
        )
        port.connect { host, p -> AdbConnection.open(host, p, tls, timeoutMs = 4_000) }.use { adb ->
            kotlinx.coroutines.currentCoroutineContext().ensureActive()
            check(app.acceptHelpers) { "节点已停止，请先启动节点" }
            val nonce = app.nonces.issueLaunch()
            adb.exec(HelperHandoff.command(app.applicationInfo.sourceDir, app.packageName, app.applicationInfo.uid, certPin())).use { stream ->
                stream.write(HelperHandoff.stdinLine(nonce))
                state.value = "正在启动虚拟屏…"
                check(withTimeoutOrNull(20_000) { app.phone.connected.first { it } } == true) {
                    "虚拟屏助手启动超时，请查看 Android 日志"
                }
            }
        }
        writeUsbScript()
        state.value = "手机控制已连接，Agent 使用独立虚拟屏"
    }

    fun writeUsbScript(): File {
        val directory = app.getExternalFilesDir(null) ?: error("手机外部存储暂不可用")
        val file = File(directory, HelperHandoff.USB_SCRIPT)
        file.writeText(HelperHandoff.usbFile(app.phone.connected.value, app.applicationInfo.sourceDir, app.packageName,
            app.applicationInfo.uid, ::certPin, app.nonces::usbNonce, file.path))
        return file
    }

    @Suppress("DEPRECATION")
    private fun certPin(): String = requireNotNull(HelperHandoff.certPin(app.packageManager
        .getPackageInfo(app.packageName, PackageManager.GET_SIGNING_CERTIFICATES).signingInfo?.apkContentsSigners?.map { it.toByteArray() }))

    private fun tls(id: AdbIdentity) = AdbTls(id, KeyingMaterialExporter { socket, label, length ->
        requireNotNull(SSLSockets.exportKeyingMaterial(socket, label, null, length)) { "TLS key export unavailable" }
    })

    private suspend fun operation(block: suspend () -> Unit) = withContext(Dispatchers.IO) {
        // Ignore repeated taps rather than queueing several stale pairing codes and reconnects.
        if (!mutex.tryLock()) return@withContext
        try {
            busy.value = true
            try { block() }
            catch (e: kotlinx.coroutines.CancellationException) { throw e }
            catch (e: Exception) {
                android.util.Log.e("CraniboxAdb", "Phone setup failed", e)
                state.value = errorText(e)
            } finally { busy.value = false }
        } finally { mutex.unlock() }
    }

    private fun errorText(e: Exception): String = when (e) {
        is AdbKeyUnreadableException -> "配对钥匙无法读取，请重新配对"
        is AdbPairingException -> if (e.reason == AdbPairingException.Reason.WRONG_CODE) "配对码不正确或已过期，请打开新的配对窗口" else "配对失败：${e.reason}"
        is AdbConnectionException -> if (e.reason == AdbConnectionException.Reason.UNAUTHORIZED) "系统未授权这把钥匙，请重新配对" else "连接失败：请确认无线调试已开启，必要时手动填写连接端口"
        else -> e.message?.take(180) ?: "操作失败，请重试"
    }

    private fun wrappingKey(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey("cranibox-adb-wrap", null) as? SecretKey)?.let { return it }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(KeyGenParameterSpec.Builder("cranibox-adb-wrap", KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).build())
        return generator.generateKey()
    }
}

private class AndroidAdbFinder(private val context: Context) : AdbServiceFinder {
    override suspend fun findPorts(serviceType: String, timeoutMs: Long): List<Int> =
        findEndpoints(serviceType, timeoutMs, localAdbHosts()).map { it.port }.distinct()

    @Suppress("DEPRECATION")
    override suspend fun findEndpoints(serviceType: String, timeoutMs: Long, localHosts: List<String>): List<AdbEndpoint> {
        val manager = context.getSystemService(NsdManager::class.java)
        val endpoints = java.util.concurrent.CopyOnWriteArraySet<AdbEndpoint>()
        val multicast = context.getSystemService(WifiManager::class.java).createMulticastLock("cranibox-adb").apply { setReferenceCounted(false) }
        val active = java.util.concurrent.atomic.AtomicBoolean(true)
        val discovery = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(type: String) {}
            override fun onDiscoveryStopped(type: String) {}
            override fun onStartDiscoveryFailed(type: String, error: Int) {}
            override fun onStopDiscoveryFailed(type: String, error: Int) {}
            override fun onServiceLost(service: NsdServiceInfo) {}
            override fun onServiceFound(service: NsdServiceInfo) {
                if (!active.get()) return
                manager.resolveService(service, object : NsdManager.ResolveListener {
                    override fun onResolveFailed(service: NsdServiceInfo, error: Int) {}
                    override fun onServiceResolved(service: NsdServiceInfo) {
                        val host = service.host ?: return
                        val local = host.isLoopbackAddress || runCatching { NetworkInterface.getByInetAddress(host) != null }.getOrDefault(false)
                        if (active.get() && local && service.port in 1..65535) {
                            host.hostAddress?.let { endpoints.add(AdbEndpoint(it, service.port)) }
                            endpoints.add(AdbEndpoint("127.0.0.1", service.port))
                        }
                    }
                })
            }
        }
        try {
            multicast.acquire()
            manager.discoverServices(serviceType, NsdManager.PROTOCOL_DNS_SD, discovery)
            delay(timeoutMs)
            return endpoints.toList()
        } finally {
            active.set(false)
            runCatching { manager.stopServiceDiscovery(discovery) }
            if (multicast.isHeld) multicast.release()
        }
    }
}

/** Only addresses assigned to this phone; never probe another device on the Wi-Fi network. */
private fun localAdbHosts(): List<String> = listOf("127.0.0.1") + runCatching {
    NetworkInterface.getNetworkInterfaces().toList().filter { it.isUp && !it.isLoopback }
        .flatMap { it.inetAddresses.toList() }.filterIsInstance<Inet4Address>()
        .mapNotNull { it.hostAddress }.distinct()
}.getOrDefault(emptyList())
