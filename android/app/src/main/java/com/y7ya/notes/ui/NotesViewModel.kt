package com.y7ya.notes.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.Companion.APPLICATION_KEY
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import com.y7ya.notes.NotesApp
import com.y7ya.notes.data.Notebook
import com.y7ya.notes.data.NotesRepository
import com.y7ya.notes.data.RECENTLY_DELETED
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import uniffi.notes_ffi.NoteRecord
import uniffi.notes_ffi.PeerRecord

data class FolderItem(val id: Long, val name: String, val count: Int)

data class FoldersState(
    val folders: List<FolderItem> = emptyList(),
    val deletedCount: Int = 0,
    val loaded: Boolean = false,
)

/** Folder and note-list operations. Shared by the folders and notes list screens. */
class NotesViewModel(private val repo: NotesRepository) : ViewModel() {
    val notebook: StateFlow<Notebook> = repo.notebook
    val peers: StateFlow<List<PeerRecord>> = repo.peers

    val folders: StateFlow<FoldersState> = repo.notebook
        .map(::foldersState)
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), foldersState(repo.notebook.value))

    fun createFolder(name: String) = repo.createFolder(name)
    fun renameFolder(id: Long, name: String) = repo.renameFolder(id, name)
    fun deleteFolder(id: Long) = repo.deleteFolder(id)

    fun setPinned(id: Long, pinned: Boolean) = repo.setPinned(id, pinned)
    fun moveNote(id: Long, folder: Long) = repo.moveNote(id, folder)
    fun trashNote(id: Long) = repo.trashNote(id)
    fun recoverNote(id: Long) = repo.recoverNote(id)
    fun deleteForever(id: Long) = repo.removeNote(id)

    /** An id for a note that the editor will create once it has text. */
    suspend fun newNoteId(): Long = repo.newId()

    companion object {
        val Factory = viewModelFactory {
            initializer { NotesViewModel((this[APPLICATION_KEY] as NotesApp).repository) }
        }
    }
}

private fun foldersState(nb: Notebook): FoldersState {
    val live = nb.notes.filter { it.deletedAt == null }
    val counts = live.groupingBy { it.folder }.eachCount()
    return FoldersState(
        folders = nb.folders.map { FolderItem(it.id, it.name, counts[it.id] ?: 0) },
        deletedCount = nb.notes.size - live.size,
        loaded = nb.loaded,
    )
}

fun Notebook.folderName(id: Long): String = when (id) {
    RECENTLY_DELETED -> "Recently Deleted"
    else -> folders.find { it.id == id }?.name ?: "Notes"
}

/** Notes shown in a folder (or Recently Deleted), filtered by a search query. */
fun Notebook.notesIn(folder: Long, query: String): List<NoteRecord> {
    val inFolder = if (folder == RECENTLY_DELETED) {
        notes.filter { it.deletedAt != null }
    } else {
        notes.filter { it.deletedAt == null && it.folder == folder }
    }
    val q = query.trim()
    return if (q.isEmpty()) inFolder else inFolder.filter { it.body.contains(q, ignoreCase = true) }
}
