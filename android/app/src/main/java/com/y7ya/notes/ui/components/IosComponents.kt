package com.y7ya.notes.ui.components

import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.windowInsetsBottomHeight
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.y7ya.notes.ui.theme.IosType
import com.y7ya.notes.ui.theme.NotesTheme

/** One physical pixel. */
val hairline: Dp @Composable get() = with(LocalDensity.current) { 1f.toDp() }

@Composable
fun Hairline(modifier: Modifier = Modifier, color: Color = NotesTheme.colors.separator) {
    Box(modifier.fillMaxWidth().height(hairline).background(color))
}

/** True once the large title has scrolled under the navigation bar. */
@Composable
fun LazyListState.rememberTitleCollapsed(): State<Boolean> {
    val threshold = with(LocalDensity.current) { 36.dp.toPx() }
    return remember(this) {
        derivedStateOf { firstVisibleItemIndex > 0 || firstVisibleItemScrollOffset > threshold }
    }
}

// -- Navigation bar ----------------------------------------------------------

/**
 * iOS navigation bar: transparent over the large title, then a translucent bar with
 * a hairline and a small centered title once the content scrolls under it.
 */
@Composable
fun IosTopBar(
    title: String,
    collapsed: Boolean,
    modifier: Modifier = Modifier,
    background: Color = NotesTheme.colors.background,
    navigation: @Composable () -> Unit = {},
    actions: @Composable RowScope.() -> Unit = {},
) {
    val colors = NotesTheme.colors
    val barColor by animateColorAsState(if (collapsed) colors.bar else background, label = "bar")
    val titleAlpha by animateFloatAsState(if (collapsed) 1f else 0f, label = "title")
    Column(modifier.fillMaxWidth().background(barColor)) {
        Box(
            Modifier.fillMaxWidth().statusBarsPadding().height(44.dp).padding(horizontal = 4.dp),
        ) {
            Text(
                title,
                style = IosType.headline,
                color = colors.label,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.align(Alignment.Center).padding(horizontal = 96.dp).alpha(titleAlpha),
            )
            Box(Modifier.align(Alignment.CenterStart)) { navigation() }
            Row(Modifier.align(Alignment.CenterEnd), verticalAlignment = Alignment.CenterVertically, content = actions)
        }
        Hairline(Modifier.alpha(titleAlpha))
    }
}

/** A borderless bar button that dims while pressed, like UIBarButtonItem. */
@Composable
fun BarButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    content: @Composable RowScope.() -> Unit,
) {
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    Row(
        modifier
            .heightIn(min = 44.dp)
            .alpha(if (!enabled) 0.35f else if (pressed) 0.3f else 1f)
            .clickable(source, indication = null, enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        content = content,
    )
}

@Composable
fun BarTextButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    bold: Boolean = false,
    enabled: Boolean = true,
) {
    BarButton(onClick, modifier, enabled = enabled) {
        Text(text, style = if (bold) IosType.headline else IosType.body, color = NotesTheme.colors.accent)
    }
}

@Composable
fun BarIconButton(
    icon: ImageVector,
    contentDescription: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    badge: Color? = null,
) {
    BarButton(onClick, modifier, enabled = enabled) {
        Box {
            Icon(icon, contentDescription, tint = NotesTheme.colors.accent, modifier = Modifier.size(26.dp))
            if (badge != null) {
                Box(Modifier.align(Alignment.TopEnd).size(8.dp).clip(RoundedCornerShape(50)).background(badge))
            }
        }
    }
}

/** "‹ Folders" */
@Composable
fun BackButton(label: String, onClick: () -> Unit) {
    BarButton(onClick) {
        Icon(IosIcons.ChevronBack, contentDescription = null, tint = NotesTheme.colors.accent, modifier = Modifier.size(24.dp))
        Text(
            label,
            style = IosType.body,
            color = NotesTheme.colors.accent,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.widthIn(max = 160.dp),
        )
    }
}

@Composable
fun LargeTitle(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = IosType.largeTitle,
        color = NotesTheme.colors.label,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 2.dp, bottom = 8.dp),
    )
}

// -- Bottom toolbar ----------------------------------------------------------

@Composable
fun BottomToolbar(modifier: Modifier = Modifier, content: @Composable BoxScope.() -> Unit) {
    Column(modifier.fillMaxWidth().background(NotesTheme.colors.bar)) {
        Hairline()
        Box(Modifier.fillMaxWidth().height(46.dp).padding(horizontal = 8.dp), content = content)
        Spacer(Modifier.windowInsetsBottomHeight(WindowInsets.navigationBars))
    }
}

// -- Search ----------------------------------------------------------------------

@Composable
fun SearchField(query: String, onQueryChange: (String) -> Unit, modifier: Modifier = Modifier) {
    val colors = NotesTheme.colors
    val focusManager = LocalFocusManager.current
    Row(
        modifier
            .padding(horizontal = 16.dp)
            .fillMaxWidth()
            .height(36.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(colors.fill)
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(IosIcons.Search, contentDescription = null, tint = colors.secondaryLabel, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(6.dp))
        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
            if (query.isEmpty()) Text("Search", style = IosType.body, color = colors.secondaryLabel)
            BasicTextField(
                value = query,
                onValueChange = onQueryChange,
                singleLine = true,
                textStyle = IosType.body.copy(color = colors.label),
                cursorBrush = SolidColor(colors.accent),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { focusManager.clearFocus() }),
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (query.isNotEmpty()) {
            ClearButton(onClick = { onQueryChange("") })
        }
    }
}

/** xmark.circle.fill */
@Composable
private fun ClearButton(onClick: () -> Unit) {
    val colors = NotesTheme.colors
    Canvas(
        Modifier
            .padding(start = 6.dp)
            .size(17.dp)
            .clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button, onClick = onClick),
    ) {
        drawCircle(colors.secondaryLabel)
        val r = size.minDimension * 0.2f
        val c = center
        val stroke = size.minDimension * 0.11f
        val x = if (colors.isDark) Color(0xFF2C2C2E) else Color.White
        drawLine(x, Offset(c.x - r, c.y - r), Offset(c.x + r, c.y + r), stroke, StrokeCap.Round)
        drawLine(x, Offset(c.x + r, c.y - r), Offset(c.x - r, c.y + r), stroke, StrokeCap.Round)
    }
}

// -- Inset grouped lists ------------------------------------------------------

enum class GroupPosition { Single, First, Middle, Last;
    val showsSeparator get() = this == First || this == Middle

    companion object {
        fun of(index: Int, count: Int) = when {
            count == 1 -> Single
            index == 0 -> First
            index == count - 1 -> Last
            else -> Middle
        }
    }
}

private const val CardRadius = 10

fun GroupPosition.shape(): Shape {
    val r = CardRadius.dp
    return when (this) {
        GroupPosition.Single -> RoundedCornerShape(r)
        GroupPosition.First -> RoundedCornerShape(topStart = r, topEnd = r)
        GroupPosition.Last -> RoundedCornerShape(bottomStart = r, bottomEnd = r)
        GroupPosition.Middle -> RoundedCornerShape(0.dp)
    }
}

/** Places a row in an inset grouped card (16dp margins, rounded ends). */
@Composable
fun Modifier.groupedRow(position: GroupPosition): Modifier =
    this.padding(horizontal = 16.dp).clip(position.shape()).background(NotesTheme.colors.card)

@Composable
fun SectionHeader(title: String, modifier: Modifier = Modifier) {
    Text(
        title,
        style = IosType.sectionHeader,
        color = NotesTheme.colors.label,
        modifier = modifier.fillMaxWidth().padding(start = 20.dp, end = 20.dp, top = 22.dp, bottom = 8.dp),
    )
}

@Composable
fun SectionFooter(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = IosType.footnote,
        color = NotesTheme.colors.secondaryLabel,
        modifier = modifier.fillMaxWidth().padding(start = 32.dp, end = 32.dp, top = 8.dp, bottom = 8.dp),
    )
}

/**
 * A row of an inset grouped list: optional leading icon, title, trailing content and
 * a hairline separator that starts at the text.
 */
@Composable
fun GroupedRow(
    position: GroupPosition,
    modifier: Modifier = Modifier,
    onClick: (() -> Unit)? = null,
    leading: (@Composable () -> Unit)? = null,
    trailing: @Composable RowScope.() -> Unit = {},
    content: @Composable RowScope.() -> Unit,
) {
    Box(modifier.groupedRow(position).let { if (onClick != null) it.clickable(onClick = onClick) else it }) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(start = 16.dp, end = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (leading != null) {
                leading()
                Spacer(Modifier.width(12.dp))
            }
            Row(Modifier.weight(1f).padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically, content = content)
            trailing()
        }
        if (position.showsSeparator) {
            Hairline(Modifier.align(Alignment.BottomStart).padding(start = if (leading != null) 54.dp else 16.dp))
        }
    }
}

@Composable
fun Chevron() {
    Icon(
        IosIcons.ChevronForward,
        contentDescription = null,
        tint = NotesTheme.colors.tertiaryLabel,
        modifier = Modifier.padding(start = 6.dp).size(14.dp),
    )
}

// -- Alerts ------------------------------------------------------------------------

enum class AlertButtonStyle { Default, Cancel, Destructive }

data class AlertButton(
    val text: String,
    val style: AlertButtonStyle = AlertButtonStyle.Default,
    val enabled: Boolean = true,
    val onClick: () -> Unit,
)

/** UIAlertController-style alert: centered title and message, buttons split by hairlines. */
@Composable
fun IosAlert(
    title: String,
    onDismiss: () -> Unit,
    buttons: List<AlertButton>,
    message: String? = null,
    field: (@Composable () -> Unit)? = null,
) {
    val colors = NotesTheme.colors
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Column(Modifier.width(270.dp).clip(RoundedCornerShape(14.dp)).background(colors.alert)) {
            Column(
                Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 19.dp, bottom = 17.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                Text(title, style = IosType.headline, color = colors.label, textAlign = TextAlign.Center)
                if (message != null) {
                    Text(message, style = IosType.footnote, color = colors.label, textAlign = TextAlign.Center)
                }
                if (field != null) {
                    Spacer(Modifier.height(10.dp))
                    field()
                }
            }
            Hairline()
            if (buttons.size == 2) {
                Row(Modifier.fillMaxWidth().height(44.dp)) {
                    AlertButtonView(buttons[0], Modifier.weight(1f))
                    Box(Modifier.width(hairline).fillMaxHeight().background(colors.separator))
                    AlertButtonView(buttons[1], Modifier.weight(1f))
                }
            } else {
                buttons.forEachIndexed { i, button ->
                    if (i > 0) Hairline()
                    AlertButtonView(button, Modifier.fillMaxWidth().height(44.dp))
                }
            }
        }
    }
}

@Composable
private fun AlertButtonView(button: AlertButton, modifier: Modifier) {
    val colors = NotesTheme.colors
    val color = when {
        !button.enabled -> colors.secondaryLabel
        button.style == AlertButtonStyle.Destructive -> colors.red
        else -> colors.accent
    }
    Box(
        modifier.clickable(enabled = button.enabled, role = Role.Button, onClick = button.onClick),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            button.text,
            style = IosType.body.copy(fontWeight = if (button.style == AlertButtonStyle.Cancel) FontWeight.Normal else FontWeight.SemiBold),
            color = color,
        )
    }
}

/** The text field inside an alert. Focuses itself so the keyboard comes up. */
@Composable
fun AlertTextField(
    value: TextFieldValue,
    onValueChange: (TextFieldValue) -> Unit,
    placeholder: String,
    onDone: () -> Unit,
) {
    val colors = NotesTheme.colors
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Box(
        Modifier
            .fillMaxWidth()
            .height(32.dp)
            .clip(RoundedCornerShape(7.dp))
            .background(colors.card)
            .border(hairline, colors.separator, RoundedCornerShape(7.dp))
            .padding(horizontal = 7.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        if (value.text.isEmpty()) Text(placeholder, style = IosType.subhead, color = colors.secondaryLabel)
        BasicTextField(
            value = value,
            onValueChange = onValueChange,
            singleLine = true,
            textStyle = IosType.subhead.copy(color = colors.label),
            cursorBrush = SolidColor(colors.accent),
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { onDone() }),
            modifier = Modifier.fillMaxWidth().focusRequester(focus),
        )
    }
}

/** Alert asking for a folder name ("New Folder" / "Rename Folder"). */
@Composable
fun FolderNameAlert(
    title: String,
    initialName: String,
    confirmText: String,
    isTaken: (String) -> Boolean,
    onDismiss: () -> Unit,
    onConfirm: (String) -> Unit,
) {
    var value by rememberSaveable(stateSaver = TextFieldValue.Saver) {
        mutableStateOf(TextFieldValue(initialName, TextRange(0, initialName.length)))
    }
    val name = value.text.trim()
    val valid = name.isNotEmpty() && (name == initialName || !isTaken(name))
    IosAlert(
        title = title,
        message = if (name.isNotEmpty() && !valid) "That name is already taken." else "Enter a name for this folder.",
        onDismiss = onDismiss,
        field = {
            AlertTextField(value, { value = it }, placeholder = "Name", onDone = { if (valid) onConfirm(name) })
        },
        buttons = listOf(
            AlertButton("Cancel", AlertButtonStyle.Cancel, onClick = onDismiss),
            AlertButton(confirmText, enabled = valid, onClick = { onConfirm(name) }),
        ),
    )
}

// -- Menus -------------------------------------------------------------------------

/** A context menu in the style of UIMenu: rounded card, text left, icon right. */
@Composable
fun IosMenu(expanded: Boolean, onDismiss: () -> Unit, content: @Composable ColumnScope.() -> Unit) {
    DropdownMenu(
        expanded = expanded,
        onDismissRequest = onDismiss,
        shape = RoundedCornerShape(13.dp),
        containerColor = NotesTheme.colors.alert,
        tonalElevation = 0.dp,
        shadowElevation = 16.dp,
        modifier = Modifier.width(250.dp),
        content = content,
    )
}

@Composable
fun IosMenuItem(
    text: String,
    icon: ImageVector,
    onClick: () -> Unit,
    destructive: Boolean = false,
    divider: Boolean = true,
) {
    val colors = NotesTheme.colors
    val color = if (destructive) colors.red else colors.label
    Column {
        Row(
            Modifier.fillMaxWidth().clickable(onClick = onClick).heightIn(min = 44.dp).padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(text, style = IosType.body, color = color, modifier = Modifier.weight(1f))
            Icon(icon, contentDescription = null, tint = color, modifier = Modifier.size(20.dp))
        }
        if (divider) Hairline()
    }
}
