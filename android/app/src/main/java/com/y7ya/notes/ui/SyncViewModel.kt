package com.y7ya.notes.ui

import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider.AndroidViewModelFactory.Companion.APPLICATION_KEY
import androidx.lifecycle.viewModelScope
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import com.y7ya.notes.NotesApp
import com.y7ya.notes.data.NotesRepository
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.notes_ffi.PairException
import uniffi.notes_ffi.PeerRecord

private const val PAIR_PREFIX = "notes-pair:1;"

sealed interface PairStatus {
    data object Idle : PairStatus
    data object Pairing : PairStatus
    data class Paired(val name: String) : PairStatus
    data class Failed(val message: String) : PairStatus
}

class SyncViewModel(private val repo: NotesRepository) : ViewModel() {
    val peers: StateFlow<List<PeerRecord>> = repo.peers

    private val _status = MutableStateFlow<PairStatus>(PairStatus.Idle)
    val status: StateFlow<PairStatus> = _status.asStateFlow()

    init {
        repo.refreshPeers()
    }

    fun pair(code: String) {
        if (_status.value == PairStatus.Pairing) return
        val trimmed = code.trim()
        if (!trimmed.startsWith(PAIR_PREFIX)) {
            _status.value = PairStatus.Failed(
                "That isn't a Notes pairing code. On your computer, open Notes and click Sync with Phone to show one.",
            )
            return
        }
        _status.value = PairStatus.Pairing
        viewModelScope.launch {
            _status.value = try {
                PairStatus.Paired(repo.pair(trimmed))
            } catch (e: PairException.Failed) {
                PairStatus.Failed(e.reason)
            }
        }
    }

    fun scanFailed(message: String) {
        _status.value = PairStatus.Failed(message)
    }

    fun unpair(id: String) = repo.unpair(id)

    companion object {
        val Factory = viewModelFactory {
            initializer { SyncViewModel((this[APPLICATION_KEY] as NotesApp).repository) }
        }
    }
}
