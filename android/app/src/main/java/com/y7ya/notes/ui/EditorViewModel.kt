package com.y7ya.notes.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.Companion.APPLICATION_KEY
import androidx.lifecycle.createSavedStateHandle
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import com.y7ya.notes.NotesApp
import com.y7ya.notes.data.NotesRepository
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.launch
import uniffi.notes_ffi.NoteRecord

private const val AUTOSAVE_DELAY_MS = 400L

/**
 * Edits one note. The text autosaves 400 ms after typing stops and when the editor
 * closes; a note left empty is discarded. Changes that arrive from the computer
 * replace the text, keeping the cursor where it was.
 */
class EditorViewModel(private val repo: NotesRepository, handle: SavedStateHandle) : ViewModel() {
    val noteId: Long = checkNotNull(handle[ARG_NOTE])

    /** Folder a new note goes into; also the folder the editor was opened from. */
    val folderId: Long = checkNotNull(handle[ARG_FOLDER])

    // Compose state rather than a StateFlow: text field state must update synchronously.
    var text by mutableStateOf(TextFieldValue())
        private set

    /** False until the notebook has loaded and [text] holds the note. */
    var ready by mutableStateOf(false)
        private set

    /** True if the note didn't exist yet when the editor opened. */
    var isNew by mutableStateOf(false)
        private set

    private val _note = MutableStateFlow<NoteRecord?>(null)

    /** The note as stored (null for a new note until it has text). */
    val note: StateFlow<NoteRecord?> = _note.asStateFlow()

    private var lastSaved: String? = null
    private var saveJob: Job? = null

    init {
        viewModelScope.launch {
            repo.notebook.filter { it.loaded }.collect { nb ->
                val stored = nb.notes.find { it.id == noteId }
                _note.value = stored
                if (!ready) {
                    isNew = stored == null
                    if (stored != null) {
                        text = TextFieldValue(stored.body)
                        lastSaved = stored.body
                    }
                    ready = true
                } else if (stored != null && stored.body != lastSaved) {
                    // Changed elsewhere (e.g. merged in from the computer).
                    lastSaved = stored.body
                    if (stored.body != text.text) {
                        val cursor = text.selection.start.coerceIn(0, stored.body.length)
                        text = TextFieldValue(stored.body, TextRange(cursor))
                    }
                }
            }
        }
    }

    val isReadOnly: Boolean get() = _note.value?.deletedAt != null

    fun onTextChange(value: TextFieldValue) {
        if (!ready || isReadOnly) return
        val changed = value.text != text.text
        text = value
        if (changed) {
            saveJob?.cancel()
            saveJob = viewModelScope.launch {
                delay(AUTOSAVE_DELAY_MS)
                save()
            }
        }
    }

    /** Saves pending edits now (e.g. when the app goes to the background). */
    fun flush() {
        saveJob?.cancel()
        save()
    }

    private fun save() {
        if (!ready || isReadOnly) return
        val body = text.text
        if (body == lastSaved) return
        // Don't create a note until it has some text.
        if (body.isBlank() && _note.value == null) return
        lastSaved = body
        repo.saveNote(noteId, folderId, body)
    }

    /** Moves the note to Recently Deleted (or drops it if it's empty). */
    fun trash() {
        saveJob?.cancel()
        if (text.text.isBlank()) {
            if (_note.value != null) repo.removeNote(noteId)
        } else {
            save()
            repo.trashNote(noteId)
        }
    }

    fun recover() = repo.recoverNote(noteId)

    override fun onCleared() {
        saveJob?.cancel()
        if (!ready) return
        val stored = repo.notebook.value.notes.find { it.id == noteId }
        if (text.text.isBlank()) {
            // Like Notes, a note left empty disappears.
            if (stored != null && stored.deletedAt == null) repo.removeNote(noteId)
        } else {
            save()
        }
    }

    companion object {
        const val ARG_NOTE = "noteId"
        const val ARG_FOLDER = "folderId"

        val Factory = viewModelFactory {
            initializer {
                EditorViewModel((this[APPLICATION_KEY] as NotesApp).repository, createSavedStateHandle())
            }
        }
    }
}
