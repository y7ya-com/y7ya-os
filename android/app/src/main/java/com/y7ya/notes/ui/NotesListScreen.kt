package com.y7ya.notes.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.y7ya.notes.data.RECENTLY_DELETED
import com.y7ya.notes.data.listDate
import com.y7ya.notes.data.preview
import com.y7ya.notes.data.sectionNotes
import com.y7ya.notes.data.title
import com.y7ya.notes.ui.components.BackButton
import com.y7ya.notes.ui.components.BarIconButton
import com.y7ya.notes.ui.components.BarTextButton
import com.y7ya.notes.ui.components.BottomToolbar
import com.y7ya.notes.ui.components.GroupPosition
import com.y7ya.notes.ui.components.GroupedRow
import com.y7ya.notes.ui.components.Hairline
import com.y7ya.notes.ui.components.IosIcons
import com.y7ya.notes.ui.components.IosMenu
import com.y7ya.notes.ui.components.IosMenuItem
import com.y7ya.notes.ui.components.IosTopBar
import com.y7ya.notes.ui.components.LargeTitle
import com.y7ya.notes.ui.components.SearchField
import com.y7ya.notes.ui.components.SectionFooter
import com.y7ya.notes.ui.components.SectionHeader
import com.y7ya.notes.ui.components.SwipeAction
import com.y7ya.notes.ui.components.SwipeableRow
import com.y7ya.notes.ui.components.groupedRow
import com.y7ya.notes.ui.components.rememberTitleCollapsed
import com.y7ya.notes.ui.theme.IosType
import com.y7ya.notes.ui.theme.NotesTheme
import uniffi.notes_ffi.NoteRecord

@Composable
fun NotesListScreen(
    viewModel: NotesViewModel,
    folderId: Long,
    onBack: () -> Unit,
    onOpenNote: (Long) -> Unit,
    onNewNote: () -> Unit,
) {
    val notebook by viewModel.notebook.collectAsStateWithLifecycle()
    val colors = NotesTheme.colors
    val isTrash = folderId == RECENTLY_DELETED
    var query by rememberSaveable { mutableStateOf("") }

    val title = notebook.folderName(folderId)
    val total = remember(notebook, folderId) { notebook.notesIn(folderId, "").size }
    val sections = remember(notebook, folderId, query) {
        sectionNotes(notebook.notesIn(folderId, query), pinnedSection = !isTrash)
    }

    val listState = rememberLazyListState()
    val collapsed by listState.rememberTitleCollapsed()
    var openRow by remember { mutableStateOf<Any?>(null) }
    var moving by remember { mutableStateOf<NoteRecord?>(null) }
    LaunchedEffect(listState.isScrollInProgress) {
        if (listState.isScrollInProgress) openRow = null
    }

    // The folder was deleted (perhaps on the computer): go back to Folders.
    val folderGone = notebook.loaded && !isTrash && notebook.folders.none { it.id == folderId }
    LaunchedEffect(folderGone) { if (folderGone) onBack() }

    val top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding()
    val bottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()

    Box(Modifier.fillMaxSize().background(colors.background)) {
        LazyColumn(
            state = listState,
            contentPadding = PaddingValues(top = top + 44.dp, bottom = bottom + 46.dp + 32.dp),
            modifier = Modifier.fillMaxSize(),
        ) {
            item(key = "title") { LargeTitle(title) }
            item(key = "search") { SearchField(query, { query = it }, Modifier.padding(bottom = 6.dp)) }
            if (isTrash && total > 0) {
                item(key = "info") {
                    SectionFooter(
                        "Notes are available here for 30 days. After that time, notes will be permanently deleted.",
                        Modifier.padding(top = 4.dp),
                    )
                }
            }
            for (section in sections) {
                item(key = "header:${section.title}") { SectionHeader(section.title, Modifier.animateItem()) }
                itemsIndexed(section.notes, key = { _, note -> note.id }) { i, note ->
                    NoteRow(
                        note = note,
                        position = GroupPosition.of(i, section.notes.size),
                        isTrash = isTrash,
                        openRow = openRow,
                        onOpenRowChange = { openRow = it },
                        onClick = { onOpenNote(note.id) },
                        onTogglePin = { viewModel.setPinned(note.id, !note.pinned) },
                        onMove = { moving = note },
                        onDelete = {
                            if (isTrash) viewModel.deleteForever(note.id) else viewModel.trashNote(note.id)
                        },
                        onRecover = { viewModel.recoverNote(note.id) },
                        modifier = Modifier.animateItem(),
                    )
                }
            }
        }

        if (notebook.loaded && sections.isEmpty()) {
            Text(
                if (query.isBlank()) "No Notes" else "No Results",
                style = IosType.title2,
                color = colors.secondaryLabel,
                modifier = Modifier.align(Alignment.Center),
            )
        }

        IosTopBar(
            title = title,
            collapsed = collapsed,
            navigation = { BackButton("Folders", onBack) },
        )

        BottomToolbar(Modifier.align(Alignment.BottomCenter)) {
            Text(
                when (total) {
                    0 -> "No Notes"
                    1 -> "1 Note"
                    else -> "$total Notes"
                },
                style = IosType.caption,
                color = colors.label,
                modifier = Modifier.align(Alignment.Center),
            )
            if (!isTrash) {
                BarIconButton(
                    IosIcons.Compose,
                    contentDescription = "New Note",
                    onClick = onNewNote,
                    modifier = Modifier.align(Alignment.CenterEnd),
                )
            }
        }
    }

    moving?.let { note ->
        MoveToFolderSheet(
            viewModel = viewModel,
            currentFolder = note.folder,
            onDismiss = { moving = null },
            onSelect = { folder ->
                viewModel.moveNote(note.id, folder)
                moving = null
            },
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun NoteRow(
    note: NoteRecord,
    position: GroupPosition,
    isTrash: Boolean,
    openRow: Any?,
    onOpenRowChange: (Any?) -> Unit,
    onClick: () -> Unit,
    onTogglePin: () -> Unit,
    onMove: () -> Unit,
    onDelete: () -> Unit,
    onRecover: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = NotesTheme.colors
    val haptics = LocalHapticFeedback.current
    var menuOpen by remember { mutableStateOf(false) }

    val leading = if (isTrash) {
        SwipeAction(IosIcons.Recover, "Recover", colors.accent, onRecover)
    } else {
        SwipeAction(if (note.pinned) IosIcons.Unpin else IosIcons.Pin, if (note.pinned) "Unpin" else "Pin", colors.accent, onTogglePin)
    }
    val trailing = SwipeAction(IosIcons.Trash, if (isTrash) "Delete Permanently" else "Delete", colors.red, onDelete)

    Box(modifier) {
        SwipeableRow(
            key = note.id,
            openKey = openRow,
            onOpenKeyChange = onOpenRowChange,
            modifier = Modifier.groupedRow(position),
            leading = leading,
            trailing = trailing,
        ) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .background(colors.card)
                    .combinedClickable(
                        onClick = onClick,
                        onLongClick = {
                            haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                            menuOpen = true
                        },
                    ),
            ) {
                Column(Modifier.padding(horizontal = 16.dp, vertical = 11.dp)) {
                    Text(
                        note.title,
                        style = IosType.headline,
                        color = colors.label,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Spacer(Modifier.height(2.dp))
                    Text(
                        buildAnnotatedString {
                            withStyle(SpanStyle(color = colors.label)) { append(listDate(note.modified)) }
                            append("  ")
                            append(note.preview)
                        },
                        style = IosType.subhead,
                        color = colors.secondaryLabel,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                if (position.showsSeparator) {
                    Hairline(Modifier.align(Alignment.BottomStart).padding(start = 16.dp))
                }
            }
        }

        IosMenu(expanded = menuOpen, onDismiss = { menuOpen = false }) {
            if (isTrash) {
                IosMenuItem("Recover", IosIcons.Recover, onClick = { menuOpen = false; onRecover() })
                IosMenuItem(
                    "Delete Permanently",
                    IosIcons.Trash,
                    onClick = { menuOpen = false; onDelete() },
                    destructive = true,
                    divider = false,
                )
            } else {
                IosMenuItem(
                    if (note.pinned) "Unpin Note" else "Pin Note",
                    if (note.pinned) IosIcons.Unpin else IosIcons.Pin,
                    onClick = { menuOpen = false; onTogglePin() },
                )
                IosMenuItem("Move to Folder…", IosIcons.Folder, onClick = { menuOpen = false; onMove() })
                IosMenuItem("Delete", IosIcons.Trash, onClick = { menuOpen = false; onDelete() }, destructive = true, divider = false)
            }
        }
    }
}

/** The "Select a folder" sheet used by Move to Folder. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MoveToFolderSheet(
    viewModel: NotesViewModel,
    currentFolder: Long,
    onDismiss: () -> Unit,
    onSelect: (Long) -> Unit,
) {
    val state by viewModel.folders.collectAsStateWithLifecycle()
    val colors = NotesTheme.colors
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = colors.background,
        dragHandle = null,
    ) {
        Box(Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 8.dp)) {
            BarTextButton("Cancel", onClick = onDismiss, modifier = Modifier.align(Alignment.CenterStart))
            Text(
                "Select a folder",
                style = IosType.headline,
                color = colors.label,
                textAlign = TextAlign.Center,
                modifier = Modifier.align(Alignment.Center),
            )
        }
        LazyColumn(contentPadding = PaddingValues(top = 8.dp, bottom = 32.dp)) {
            itemsIndexed(state.folders, key = { _, f -> f.id }) { i, folder ->
                val current = folder.id == currentFolder
                GroupedRow(
                    position = GroupPosition.of(i, state.folders.size),
                    onClick = if (current) null else ({ onSelect(folder.id) }),
                    leading = {
                        Icon(
                            IosIcons.Folder,
                            contentDescription = null,
                            tint = if (current) colors.secondaryLabel else colors.accent,
                            modifier = Modifier.size(26.dp),
                        )
                    },
                    trailing = {
                        Text(folder.count.toString(), style = IosType.body, color = colors.secondaryLabel)
                    },
                ) {
                    Text(
                        folder.name,
                        style = IosType.body,
                        color = if (current) colors.secondaryLabel else colors.label,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}
