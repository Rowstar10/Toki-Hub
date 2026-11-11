package com.rowstar10.tokihub.device

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File

@InvokeArg
class NameArgs {
    lateinit var name: String
}

@InvokeArg
class FileArgs {
    lateinit var path: String
}

/** Toki Hub's games are apps: this asks Android to install, start and remove them. */
@TauriPlugin
class DevicePlugin(private val activity: Activity) : Plugin(activity) {

    @Command
    fun folders(invoke: Invoke) {
        val folders = JSObject()
        folders.put("files", activity.filesDir.absolutePath)
        folders.put("cache", activity.cacheDir.absolutePath)
        invoke.resolve(folders)
    }

    /** The installed app's version name, or null. */
    @Command
    fun installed(invoke: Invoke) {
        val args = invoke.parseArgs(NameArgs::class.java)
        val answer = JSObject()
        try {
            val info = activity.packageManager.getPackageInfo(args.name, 0)
            answer.put("version", info.versionName ?: "")
        } catch (e: PackageManager.NameNotFoundException) {
            answer.put("version", null as String?)   // (left out: not installed)
        }
        invoke.resolve(answer)
    }

    /** Hand an APK to Android's installer. The first time, Android asks to allow Toki Hub to. */
    @Command
    fun install(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(FileArgs::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && !activity.packageManager.canRequestPackageInstalls()) {
                val allow = Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + activity.packageName))
                activity.startActivity(allow)
                invoke.reject("Allow Toki Hub to install apps (Android has just asked), then press Install again.")
                return
            }
            val file = File(args.path)
            // the app's own file provider (src-tauri/gen/android: file_paths.xml)
            val uri = FileProvider.getUriForFile(activity, activity.packageName + ".fileprovider", file)
            val intent = Intent(Intent.ACTION_VIEW)
                .setDataAndType(uri, "application/vnd.android.package-archive")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.message ?: "Android couldn't open its installer")
        }
    }

    @Command
    fun launch(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(NameArgs::class.java)
            val intent = activity.packageManager.getLaunchIntentForPackage(args.name)
            if (intent == null) {
                invoke.reject("That game isn't installed any more.")
                return
            }
            intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.message ?: "Android couldn't start the game")
        }
    }

    /** Android asks the player, then removes the app. */
    @Command
    fun uninstall(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(NameArgs::class.java)
            val intent = Intent(Intent.ACTION_DELETE, Uri.parse("package:" + args.name))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            activity.startActivity(intent)
            invoke.resolve()
        } catch (e: Exception) {
            invoke.reject(e.message ?: "Android couldn't remove the game")
        }
    }
}
