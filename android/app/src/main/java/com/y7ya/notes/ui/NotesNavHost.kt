package com.y7ya.notes.ui

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.tween
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import com.y7ya.notes.data.DEFAULT_FOLDER
import com.y7ya.notes.ui.theme.NotesTheme
import kotlinx.coroutines.launch

private object Routes {
    const val FOLDERS = "folders"
    const val NOTES = "notes/{folderId}"
    const val EDITOR = "note/{${EditorViewModel.ARG_NOTE}}/{${EditorViewModel.ARG_FOLDER}}"
    const val SYNC = "sync"

    fun notes(folderId: Long) = "notes/$folderId"
    fun editor(noteId: Long, folderId: Long) = "note/$noteId/$folderId"
}

// UINavigationController's push: the new screen slides in over the old one, which
// shifts a little to the left. Predictive back scrubs the pop transition.
private val IosEasing = CubicBezierEasing(0.2f, 0.9f, 0.3f, 1f)
private const val PUSH_MS = 400

@Composable
fun NotesNavHost() {
    val nav = rememberNavController()
    val notesViewModel: NotesViewModel = viewModel(factory = NotesViewModel.Factory)
    val scope = rememberCoroutineScope()
    var creating by remember { mutableStateOf(false) }

    fun newNote(folderId: Long, replaceCurrent: Boolean = false) {
        if (creating) return
        creating = true
        scope.launch {
            try {
                val id = notesViewModel.newNoteId()
                nav.navigate(Routes.editor(id, folderId)) {
                    if (replaceCurrent) popUpTo(Routes.EDITOR) { inclusive = true }
                }
            } finally {
                creating = false
            }
        }
    }

    NavHost(
        navController = nav,
        startDestination = Routes.FOLDERS,
        modifier = Modifier
            .fillMaxSize()
            .background(NotesTheme.colors.background)
            // Side insets (landscape cutouts, 3-button nav); screens handle top and bottom.
            .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal)),
        enterTransition = { slideInHorizontally(tween(PUSH_MS, easing = IosEasing)) { it } },
        exitTransition = { slideOutHorizontally(tween(PUSH_MS, easing = IosEasing)) { -it / 3 } },
        popEnterTransition = { slideInHorizontally(tween(PUSH_MS, easing = IosEasing)) { -it / 3 } },
        popExitTransition = { slideOutHorizontally(tween(PUSH_MS, easing = IosEasing)) { it } },
    ) {
        composable(Routes.FOLDERS) { entry ->
            FoldersScreen(
                viewModel = notesViewModel,
                onOpenFolder = { id -> if (entry.isResumed()) nav.navigate(Routes.notes(id)) },
                onNewNote = { newNote(DEFAULT_FOLDER) },
                onOpenSync = { if (entry.isResumed()) nav.navigate(Routes.SYNC) },
            )
        }
        composable(
            Routes.NOTES,
            arguments = listOf(navArgument("folderId") { type = NavType.LongType }),
        ) { entry ->
            val folderId = entry.arguments?.getLong("folderId") ?: DEFAULT_FOLDER
            NotesListScreen(
                viewModel = notesViewModel,
                folderId = folderId,
                onBack = { if (entry.isResumed()) nav.popBackStack() },
                onOpenNote = { id -> if (entry.isResumed()) nav.navigate(Routes.editor(id, folderId)) },
                onNewNote = { newNote(folderId) },
            )
        }
        composable(
            Routes.EDITOR,
            arguments = listOf(
                navArgument(EditorViewModel.ARG_NOTE) { type = NavType.LongType },
                navArgument(EditorViewModel.ARG_FOLDER) { type = NavType.LongType },
            ),
        ) { entry ->
            val editor: EditorViewModel = viewModel(factory = EditorViewModel.Factory)
            val cameFromFolders = nav.previousBackStackEntry?.destination?.route == Routes.FOLDERS
            EditorScreen(
                viewModel = editor,
                notesViewModel = notesViewModel,
                backLabel = if (cameFromFolders) "Folders" else null,
                onBack = { if (entry.isResumed()) nav.popBackStack() },
                onNewNote = { newNote(editor.folderId, replaceCurrent = true) },
            )
        }
        composable(Routes.SYNC) { entry ->
            SyncScreen(
                viewModel = viewModel(factory = SyncViewModel.Factory),
                onBack = { if (entry.isResumed()) nav.popBackStack() },
            )
        }
    }
}

/** Ignores taps on a screen that is already animating away (e.g. a double tap on back). */
private fun NavBackStackEntry.isResumed() = lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)
