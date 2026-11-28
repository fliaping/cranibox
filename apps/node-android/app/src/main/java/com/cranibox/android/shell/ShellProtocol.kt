package com.cranibox.android.shell

import android.os.Binder
import android.os.Bundle
import android.os.IBinder
import android.os.Parcel
import android.view.Surface

/** Deliberately small Binder surface: no arbitrary shell commands or primary-display input. */
object ShellProtocol {
    const val DESCRIPTOR = "com.cranibox.android.ICraniboxShell"
    const val CALL = IBinder.FIRST_CALL_TRANSACTION
    const val IDENTITY = "identity"
    const val DISPLAY = "display"
    const val STROKE = "stroke"
    const val KEY = "key"
    const val TYPE = "type"
    const val TASKS = "tasks"
    const val FOCUS = "focus"
    const val OPEN = "open"
    const val RELEASE = "release"
    const val EXIT = "exit"
    const val PAUSE = "pause"
}

class ShellClient(val binder: IBinder) {
    fun call(action: String, args: Bundle = Bundle(), surface: Surface? = null): Bundle {
        val input = Parcel.obtain()
        val output = Parcel.obtain()
        try {
            input.writeInterfaceToken(ShellProtocol.DESCRIPTOR)
            input.writeString(action)
            input.writeBundle(args)
            input.writeParcelable(surface, 0)
            check(binder.transact(ShellProtocol.CALL, input, output, 0)) { "helper protocol unavailable" }
            output.readException()
            return output.readBundle(ShellClient::class.java.classLoader) ?: Bundle.EMPTY
        } finally {
            input.recycle()
            output.recycle()
        }
    }
}

abstract class ShellBinder(private val appUid: Int) : Binder() {
    override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
        if (code != ShellProtocol.CALL) return super.onTransact(code, data, reply, flags)
        if (getCallingUid() != appUid) throw SecurityException("only the owning app may control the helper")
        data.enforceInterface(ShellProtocol.DESCRIPTOR)
        val action = data.readString() ?: throw IllegalArgumentException("action required")
        val args = data.readBundle(ShellBinder::class.java.classLoader) ?: Bundle.EMPTY
        @Suppress("DEPRECATION")
        val surface = data.readParcelable<Surface>(ShellBinder::class.java.classLoader)
        val identity = clearCallingIdentity()
        try {
            val result = execute(action, args, surface)
            reply?.writeNoException()
            reply?.writeBundle(result)
        } finally {
            restoreCallingIdentity(identity)
            surface?.release()
        }
        return true
    }

    abstract fun execute(action: String, args: Bundle, surface: Surface?): Bundle
}
