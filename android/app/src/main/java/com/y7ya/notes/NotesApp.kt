package com.y7ya.notes

import android.app.Application
import android.net.wifi.WifiManager
import android.os.Build
import android.provider.Settings
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import com.y7ya.notes.data.NotesRepository
import java.io.File

class NotesApp : Application() {
    lateinit var repository: NotesRepository
        private set

    // The core finds the computer with mDNS, which Android filters out without this.
    private val multicastLock: WifiManager.MulticastLock? by lazy {
        getSystemService(WifiManager::class.java)
            ?.createMulticastLock("notes-mdns")
            ?.apply { setReferenceCounted(false) }
    }

    override fun onCreate() {
        super.onCreate()
        repository = NotesRepository(File(filesDir, "notes"), deviceName())

        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                multicastLock?.acquire()
                repository.syncNow()
            }

            override fun onStop(owner: LifecycleOwner) {
                multicastLock?.takeIf { it.isHeld }?.release()
            }
        })
    }

    private fun deviceName(): String =
        Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)
            ?.takeIf { it.isNotBlank() }
            ?: Build.MODEL
}
