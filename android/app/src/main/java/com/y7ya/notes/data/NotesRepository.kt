package com.y7ya.notes.data

import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.notes_ffi.CoreListener
import uniffi.notes_ffi.FolderRecord
import uniffi.notes_ffi.NoteRecord
import uniffi.notes_ffi.NotesCore
import uniffi.notes_ffi.PairException
import uniffi.notes_ffi.PeerRecord
import uniffi.notes_ffi.SnapshotRecord

/** The built-in "Notes" folder. It can't be renamed or deleted. */
const val DEFAULT_FOLDER = 0L

/** Pseudo folder id used by the UI for Recently Deleted. Real ids are never negative. */
const val RECENTLY_DELETED = -1L

private const val PURGE_AFTER_SECS = 30L * 24 * 3600

/** The whole notebook. Records are treated as immutable: change them with `copy`. */
data class Notebook(
    val folders: List<FolderRecord> = emptyList(),
    val notes: List<NoteRecord> = emptyList(),
    val loaded: Boolean = false,
)

fun nowSecs(): Long = System.currentTimeMillis() / 1000

/**
 * Owns the Rust core. Notebook calls (snapshot/apply/newId) are serialized on one
 * dispatcher. Each edit is a pure function of the notebook: it is applied to the UI
 * state right away and then, on the core dispatcher, to the version the core last
 * gave us, so `apply` always gets the full state plus the heads it was based on.
 */
class NotesRepository(dataDir: File, deviceName: String) {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    @OptIn(ExperimentalCoroutinesApi::class)
    private val notebookDispatcher = Dispatchers.IO.limitedParallelism(1)

    private val listener = object : CoreListener {
        override fun notesChanged() = requestReload()
        override fun statusChanged() = refreshPeers()
    }

    private val core: NotesCore by lazy {
        dataDir.mkdirs()
        NotesCore(dataDir.path, deviceName, listener)
    }

    private val _notebook = MutableStateFlow(Notebook())
    val notebook: StateFlow<Notebook> = _notebook.asStateFlow()

    private val _peers = MutableStateFlow<List<PeerRecord>>(emptyList())
    val peers: StateFlow<List<PeerRecord>> = _peers.asStateFlow()

    // Only touched on notebookDispatcher: the core's notebook at `heads`.
    private var heads = ByteArray(0)
    private var stored = Notebook()

    // Edits whose optimistic result is shown but not yet applied to the core.
    // While any are queued, core state isn't published, so the UI doesn't flicker back.
    private val lock = Any()
    private var pending = 0
    private val reloadQueued = AtomicBoolean(false)

    init {
        scope.launch(notebookDispatcher) {
            load()
            publishIfIdle()
            val cutoff = nowSecs() - PURGE_AFTER_SECS
            if (stored.notes.any { (it.deletedAt ?: Long.MAX_VALUE) < cutoff }) {
                edit { nb -> nb.copy(notes = nb.notes.filter { (it.deletedAt ?: Long.MAX_VALUE) >= cutoff }) }
            }
        }
        refreshPeers()
    }

    /** Applies [change] to the notebook and saves it. [change] must be pure. */
    fun edit(change: (Notebook) -> Notebook) {
        synchronized(lock) {
            pending++
            _notebook.update(change)
        }
        scope.launch(notebookDispatcher) {
            try {
                val next = change(stored)
                if (next != stored) {
                    // The result includes anything merged in from the computer.
                    adopt(core.apply(heads, next.folders, next.notes))
                }
            } finally {
                synchronized(lock) {
                    pending--
                    if (pending == 0) _notebook.value = stored
                }
            }
        }
    }

    private fun load() = adopt(core.snapshot())

    private fun adopt(snapshot: SnapshotRecord) {
        heads = snapshot.heads
        stored = Notebook(snapshot.folders, snapshot.notes, loaded = true)
    }

    private fun publishIfIdle() = synchronized(lock) {
        if (pending == 0) _notebook.value = stored
    }

    private fun requestReload() {
        if (!reloadQueued.compareAndSet(false, true)) return
        scope.launch(notebookDispatcher) {
            reloadQueued.set(false)
            load()
            publishIfIdle()
        }
    }

    suspend fun newId(): Long = withContext(notebookDispatcher) { core.newId() }

    // -- Sync. These don't touch the notebook, and pairing blocks for a while, so
    // they run on the IO pool rather than the notebook queue.

    fun refreshPeers() {
        scope.launch(Dispatchers.IO) { _peers.value = core.peers() }
    }

    fun syncNow() {
        scope.launch(Dispatchers.IO) { core.syncNow() }
    }

    /** Pairs with the computer showing [code]. Returns its name. */
    @Throws(PairException::class)
    suspend fun pair(code: String): String = withContext(Dispatchers.IO) {
        core.pair(code).also { _peers.value = core.peers() }
    }

    fun unpair(id: String) {
        scope.launch(Dispatchers.IO) {
            core.unpair(id)
            _peers.value = core.peers()
        }
    }

    // -- Folders

    fun createFolder(name: String) {
        scope.launch {
            val id = newId()
            edit { it.copy(folders = it.folders + FolderRecord(id, name)) }
        }
    }

    fun renameFolder(id: Long, name: String) {
        if (id == DEFAULT_FOLDER) return
        edit { nb -> nb.copy(folders = nb.folders.map { if (it.id == id) it.copy(name = name) else it }) }
    }

    /** Deletes a folder; its notes move to Recently Deleted (and recover into Notes). */
    fun deleteFolder(id: Long) {
        if (id == DEFAULT_FOLDER) return
        val now = nowSecs()
        edit { nb ->
            nb.copy(
                folders = nb.folders.filter { it.id != id },
                notes = nb.notes.map {
                    if (it.folder != id) it
                    else it.copy(folder = DEFAULT_FOLDER, deletedAt = it.deletedAt ?: now)
                },
            )
        }
    }

    // -- Notes

    /** Creates or updates a note's text. */
    fun saveNote(id: Long, folder: Long, body: String) {
        val now = nowSecs()
        edit { nb ->
            if (nb.notes.any { it.id == id }) {
                nb.updateNote(id) { if (it.body == body) it else it.copy(body = body, modified = now) }
            } else {
                val note = NoteRecord(
                    id = id, folder = folder.takeIf { it >= 0 } ?: DEFAULT_FOLDER, body = body,
                    created = now, modified = now, pinned = false, deletedAt = null,
                )
                nb.copy(notes = nb.notes + note)
            }
        }
    }

    fun setPinned(id: Long, pinned: Boolean) = edit { nb -> nb.updateNote(id) { it.copy(pinned = pinned) } }

    fun moveNote(id: Long, folder: Long) = edit { nb -> nb.updateNote(id) { it.copy(folder = folder) } }

    /** Moves a note to Recently Deleted. */
    fun trashNote(id: Long) {
        val now = nowSecs()
        edit { nb -> nb.updateNote(id) { it.copy(deletedAt = now, pinned = false) } }
    }

    fun recoverNote(id: Long) = edit { nb ->
        val folderIds = nb.folders.mapTo(HashSet()) { it.id }
        nb.updateNote(id) {
            it.copy(deletedAt = null, folder = if (it.folder in folderIds) it.folder else DEFAULT_FOLDER)
        }
    }

    /** Removes a note for good (from Recently Deleted, or a note left empty). */
    fun removeNote(id: Long) = edit { nb -> nb.copy(notes = nb.notes.filter { it.id != id }) }
}

private inline fun Notebook.updateNote(id: Long, change: (NoteRecord) -> NoteRecord): Notebook =
    copy(notes = notes.map { if (it.id == id) change(it) else it })
