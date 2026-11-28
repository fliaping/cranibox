package com.cranibox.android

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.Bundle
import com.cranibox.android.core.helper.HelperHandoff
import com.cranibox.android.shell.ShellClient
import com.cranibox.android.shell.ShellProtocol

/** Only the shell uid may present a helper, with a one-use credential issued by this app. */
class HelperProvider : ContentProvider() {
    private val life = Binder()
    override fun onCreate() = true

    @Synchronized
    override fun call(method: String, arg: String?, extras: Bundle?): Bundle {
        if (method != HelperHandoff.METHOD_ATTACH || extras == null) return rejected("unsupported handoff")
        val app = requireNotNull(context).node
        if (!app.acceptHelpers) return rejected("node stopped on this phone")
        val helper = extras.getBinder(HelperHandoff.KEY_BINDER)
        val pid = Binder.getCallingPid()
        val uid = Binder.getCallingUid()
        val rejection = HelperHandoff.check(uid, pid, extras.getInt(HelperHandoff.KEY_UID, -1),
            extras.getInt(HelperHandoff.KEY_PID, -1), extras.getString(HelperHandoff.KEY_APK), app.applicationInfo.sourceDir, helper != null)
        if (rejection != null) return rejected(rejection.name)
        val identity = Binder.clearCallingIdentity()
        try {
            val shell = ShellClient(requireNotNull(helper))
            val who = shell.call(ShellProtocol.IDENTITY)
            if (who.getInt("pid") != pid || who.getInt("uid") != uid || who.getString("apk") != app.applicationInfo.sourceDir) {
                return rejected(HelperHandoff.Reject.BinderElsewhere.name)
            }
            if (!app.nonces.matches(extras.getString(HelperHandoff.KEY_NONCE))) return rejected("bad nonce")
            app.phone.attach(shell)
            val next = app.nonces.accept(extras.getString(HelperHandoff.KEY_NONCE)) ?: run {
                app.phone.disconnect(true)
                return rejected("expired nonce")
            }
            return Bundle().apply {
                putBoolean(HelperHandoff.KEY_ACCEPTED, true)
                putString(HelperHandoff.KEY_NEXT, next)
                putBinder(HelperHandoff.KEY_TOKEN, life)
            }
        } catch (error: Exception) {
            android.util.Log.e("CraniboxShell", "Cannot initialize phone control", error)
            return rejected("cannot initialize phone control")
        } finally { Binder.restoreCallingIdentity(identity) }
    }

    private fun rejected(reason: String) = Bundle().apply {
        android.util.Log.w("CraniboxShell", "Helper handoff rejected: $reason")
        putBoolean(HelperHandoff.KEY_ACCEPTED, false); putString(HelperHandoff.KEY_REASON, reason)
    }
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? = null
    override fun getType(uri: Uri): String? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?) = 0
    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?) = 0
}
