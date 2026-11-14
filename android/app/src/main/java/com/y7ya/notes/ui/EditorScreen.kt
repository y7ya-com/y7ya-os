package com.y7ya.notes.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.y7ya.notes.data.editorDate
import com.y7ya.notes.data.nowSecs
import com.y7ya.notes.ui.components.AlertButton
import com.y7ya.notes.ui.components.AlertButtonStyle
import com.y7ya.notes.ui.components.BackButton
import com.y7ya.notes.ui.components.BarIconButton
import com.y7ya.notes.ui.components.BarTextButton
import com.y7ya.notes.ui.components.BottomToolbar
import com.y7ya.notes.ui.components.IosAlert
import com.y7ya.notes.ui.components.IosIcons
import com.y7ya.notes.ui.components.IosTopBar
import com.y7ya.notes.ui.theme.IosType
import com.y7ya.notes.ui.theme.NotesTheme

@OptIn(ExperimentalLayoutApi::class, ExperimentalFoundationApi::class)
@Composable
fun EditorScreen(
    viewModel: EditorViewModel,
    notesViewModel: NotesViewModel,
    backLabel: String?,
    onBack: () -> Unit,
    onNewNote: () -> Unit,
) {
    val colors = NotesTheme.colors
    val notebook by notesViewModel.notebook.collectAsStateWithLifecycle()
    val note by viewModel.note.collectAsStateWithLifecycle()
    val readOnly = note?.deletedAt != null
    val imeVisible = WindowInsets.isImeVisible
    val focusManager = LocalFocusManager.current
    val keyboard = LocalSoftwareKeyboardController.current
    val focusRequester = remember { FocusRequester() }
    var askRecover by remember { mutableStateOf(false) }

    val textStyle = IosType.body.copy(color = colors.label, lineHeight = TextUnit.Unspecified)
    val titleTransformation = remember { TitleTransformation(IosType.noteTitle.toSpanStyle()) }

    // Save right away when the app goes to the background.
    LifecycleEventEffect(Lifecycle.Event.ON_STOP) { viewModel.flush() }

    // A new note opens with the keyboard up.
    LaunchedEffect(viewModel.ready) {
        if (viewModel.ready && viewModel.isNew) focusRequester.requestFocus()
    }

    Column(Modifier.fillMaxSize().background(colors.editor).imePadding()) {
        IosTopBar(
            title = "",
            collapsed = false,
            background = colors.editor,
            navigation = { BackButton(backLabel ?: notebook.folderName(viewModel.folderId), onBack) },
            actions = {
                if (imeVisible && !readOnly) {
                    BarTextButton("Done", bold = true, onClick = {
                        focusManager.clearFocus()
                        keyboard?.hide()
                    })
                }
            },
        )

        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
            val minTextHeight = maxHeight - 48.dp
            Column(
                Modifier
                    .fillMaxSize()
                    .verticalScroll(rememberScrollState())
                    .padding(horizontal = 16.dp),
            ) {
                Text(
                    editorDate(note?.modified ?: nowSecs()),
                    style = IosType.footnote,
                    color = colors.secondaryLabel,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.fillMaxWidth().padding(top = 2.dp, bottom = 14.dp),
                )
                if (readOnly) {
                    // Recently deleted notes can't be edited; tapping offers to recover.
                    Text(
                        titleTransformation.filter(AnnotatedString(viewModel.text.text)).text,
                        style = textStyle,
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = minTextHeight)
                            .clickable(interactionSource = null, indication = null) { askRecover = true },
                    )
                } else {
                    BasicTextField(
                        value = viewModel.text,
                        onValueChange = viewModel::onTextChange,
                        enabled = viewModel.ready,
                        textStyle = textStyle,
                        cursorBrush = SolidColor(colors.accent),
                        visualTransformation = titleTransformation,
                        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(min = minTextHeight)
                            .focusRequester(focusRequester),
                    )
                }
            }
        }

        if (!imeVisible && !readOnly) {
            BottomToolbar {
                BarIconButton(
                    IosIcons.Trash,
                    contentDescription = "Delete Note",
                    onClick = {
                        viewModel.trash()
                        onBack()
                    },
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
    }

    if (askRecover) {
        IosAlert(
            title = "Recently Deleted",
            message = "Recently deleted notes can’t be edited. To edit this note, you’ll need to recover it.",
            onDismiss = { askRecover = false },
            buttons = listOf(
                AlertButton("Cancel", AlertButtonStyle.Cancel) { askRecover = false },
                AlertButton("Recover") {
                    viewModel.recover()
                    askRecover = false
                },
            ),
        )
    }
}

/** Shows the first line of the note as its title: big and bold, like Notes. */
private class TitleTransformation(private val title: SpanStyle) : VisualTransformation {
    override fun filter(text: AnnotatedString): TransformedText {
        val end = text.indexOf('\n').let { if (it < 0) text.length else it }
        val styled = buildAnnotatedString {
            append(text)
            if (end > 0) addStyle(title, 0, end)
        }
        return TransformedText(styled, OffsetMapping.Identity)
    }

    override fun equals(other: Any?) = other is TitleTransformation && other.title == title
    override fun hashCode() = title.hashCode()
}
