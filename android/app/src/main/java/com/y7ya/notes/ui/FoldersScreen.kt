package com.y7ya.notes.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.y7ya.notes.data.DEFAULT_FOLDER
import com.y7ya.notes.data.RECENTLY_DELETED
import com.y7ya.notes.ui.components.AlertButton
import com.y7ya.notes.ui.components.AlertButtonStyle
import com.y7ya.notes.ui.components.BarIconButton
import com.y7ya.notes.ui.components.BarTextButton
import com.y7ya.notes.ui.components.BottomToolbar
import com.y7ya.notes.ui.components.Chevron
import com.y7ya.notes.ui.components.FolderNameAlert
import com.y7ya.notes.ui.components.GroupPosition
import com.y7ya.notes.ui.components.GroupedRow
import com.y7ya.notes.ui.components.IosAlert
import com.y7ya.notes.ui.components.IosIcons
import com.y7ya.notes.ui.components.IosTopBar
import com.y7ya.notes.ui.components.LargeTitle
import com.y7ya.notes.ui.components.rememberTitleCollapsed
import com.y7ya.notes.ui.theme.IosType
import com.y7ya.notes.ui.theme.NotesTheme

private sealed interface FolderDialog {
    data object New : FolderDialog
    data class Rename(val id: Long, val name: String) : FolderDialog
    data class Delete(val id: Long, val name: String) : FolderDialog
}

@Composable
fun FoldersScreen(
    viewModel: NotesViewModel,
    onOpenFolder: (Long) -> Unit,
    onNewNote: () -> Unit,
    onOpenSync: () -> Unit,
) {
    val state by viewModel.folders.collectAsStateWithLifecycle()
    val peers by viewModel.peers.collectAsStateWithLifecycle()
    val colors = NotesTheme.colors
    val listState = rememberLazyListState()
    val collapsed by listState.rememberTitleCollapsed()
    var editing by rememberSaveable { mutableStateOf(false) }
    var dialog by remember { mutableStateOf<FolderDialog?>(null) }

    val top = WindowInsets.statusBars.asPaddingValues().calculateTopPadding()
    val bottom = WindowInsets.navigationBars.asPaddingValues().calculateBottomPadding()

    Box(Modifier.fillMaxSize().background(colors.background)) {
        LazyColumn(
            state = listState,
            contentPadding = PaddingValues(top = top + 44.dp, bottom = bottom + 46.dp + 32.dp),
            modifier = Modifier.fillMaxSize(),
        ) {
            item(key = "title") { LargeTitle("Folders") }
            item(key = "gap") { Spacer(Modifier.height(10.dp)) }

            if (state.loaded) {
                val showDeleted = state.deletedCount > 0
                val rows = state.folders.size + if (showDeleted) 1 else 0
                itemsIndexed(state.folders, key = { _, f -> f.id }) { i, folder ->
                    val deletable = folder.id != DEFAULT_FOLDER
                    FolderRow(
                        icon = IosIcons.Folder,
                        name = folder.name,
                        count = folder.count,
                        position = GroupPosition.of(i, rows),
                        editing = editing,
                        deletable = deletable,
                        onDelete = { dialog = FolderDialog.Delete(folder.id, folder.name) },
                        onClick = {
                            if (!editing) onOpenFolder(folder.id)
                            else if (deletable) dialog = FolderDialog.Rename(folder.id, folder.name)
                        },
                        modifier = Modifier.animateItem(),
                    )
                }
                if (showDeleted) {
                    item(key = "deleted") {
                        FolderRow(
                            icon = IosIcons.Trash,
                            name = "Recently Deleted",
                            count = state.deletedCount,
                            position = GroupPosition.of(rows - 1, rows),
                            editing = editing,
                            deletable = false,
                            onDelete = {},
                            onClick = { if (!editing) onOpenFolder(RECENTLY_DELETED) },
                            modifier = Modifier.animateItem(),
                        )
                    }
                }
            }
        }

        IosTopBar(
            title = "Folders",
            collapsed = collapsed,
            actions = {
                BarIconButton(
                    IosIcons.Computer,
                    contentDescription = "Sync with Computer",
                    onClick = onOpenSync,
                    badge = if (peers.any { it.connected }) colors.green else null,
                )
                BarTextButton(if (editing) "Done" else "Edit", onClick = { editing = !editing }, bold = editing)
            },
        )

        BottomToolbar(Modifier.align(Alignment.BottomCenter)) {
            BarIconButton(
                IosIcons.NewFolder,
                contentDescription = "New Folder",
                onClick = { dialog = FolderDialog.New },
                modifier = Modifier.align(Alignment.CenterStart),
            )
            BarIconButton(
                IosIcons.Compose,
                contentDescription = "New Note",
                onClick = onNewNote,
                modifier = Modifier.align(Alignment.CenterEnd),
            )
        }
    }

    val isTaken = { name: String -> state.folders.any { it.name.equals(name, ignoreCase = true) } }
    when (val d = dialog) {
        null -> {}
        FolderDialog.New -> FolderNameAlert(
            title = "New Folder",
            initialName = "",
            confirmText = "Save",
            isTaken = isTaken,
            onDismiss = { dialog = null },
            onConfirm = { viewModel.createFolder(it); dialog = null },
        )
        is FolderDialog.Rename -> FolderNameAlert(
            title = "Rename Folder",
            initialName = d.name,
            confirmText = "Save",
            isTaken = isTaken,
            onDismiss = { dialog = null },
            onConfirm = { viewModel.renameFolder(d.id, it); dialog = null },
        )
        is FolderDialog.Delete -> IosAlert(
            title = "Delete Folder?",
            message = "All notes in “${d.name}” will be moved to Recently Deleted.",
            onDismiss = { dialog = null },
            buttons = listOf(
                AlertButton("Cancel", AlertButtonStyle.Cancel) { dialog = null },
                AlertButton("Delete", AlertButtonStyle.Destructive) {
                    viewModel.deleteFolder(d.id)
                    dialog = null
                },
            ),
        )
    }
}

@Composable
private fun FolderRow(
    icon: ImageVector,
    name: String,
    count: Int,
    position: GroupPosition,
    editing: Boolean,
    deletable: Boolean,
    onDelete: () -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = NotesTheme.colors
    GroupedRow(
        position = position,
        modifier = modifier,
        onClick = onClick,
        leading = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                AnimatedVisibility(
                    visible = editing && deletable,
                    enter = expandHorizontally() + fadeIn(),
                    exit = shrinkHorizontally() + fadeOut(),
                ) {
                    DeleteBadge(onDelete, Modifier.padding(end = 12.dp))
                }
                Icon(icon, contentDescription = null, tint = colors.accent, modifier = Modifier.size(26.dp))
            }
        },
        trailing = {
            if (!editing) {
                Text(count.toString(), style = IosType.body, color = colors.secondaryLabel)
                Chevron()
            }
        },
    ) {
        Text(
            name,
            style = IosType.body,
            color = if (editing && !deletable) colors.secondaryLabel else colors.label,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

/** The red minus circle shown on deletable rows in edit mode. */
@Composable
private fun DeleteBadge(onClick: () -> Unit, modifier: Modifier = Modifier) {
    val red = NotesTheme.colors.red
    Canvas(
        modifier
            .size(22.dp)
            .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button, onClick = onClick),
    ) {
        drawCircle(red)
        val half = size.width * 0.27f
        drawLine(Color.White, Offset(center.x - half, center.y), Offset(center.x + half, center.y), size.width * 0.12f, StrokeCap.Round)
    }
}
