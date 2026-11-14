package com.y7ya.notes.ui.theme

import androidx.compose.foundation.IndicationNodeFactory
import androidx.compose.foundation.LocalIndication
import androidx.compose.foundation.interaction.InteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.text.selection.LocalTextSelectionColors
import androidx.compose.foundation.text.selection.TextSelectionColors
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.ContentDrawScope
import androidx.compose.ui.node.DelegatableNode
import androidx.compose.ui.node.DrawModifierNode
import androidx.compose.ui.node.invalidateDraw
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import androidx.compose.ui.Modifier
import kotlinx.coroutines.launch

val Accent = Color(0xFFE2A800)

/** iOS system colors used by Notes. */
@Immutable
data class IosColors(
    val background: Color,
    val card: Color,
    val editor: Color,
    val label: Color,
    val secondaryLabel: Color,
    val tertiaryLabel: Color,
    val separator: Color,
    val fill: Color,
    val bar: Color,
    val alert: Color,
    val highlight: Color,
    val accent: Color = Accent,
    val red: Color,
    val green: Color,
    val isDark: Boolean,
)

private val LightColors = IosColors(
    background = Color(0xFFF2F2F7),
    card = Color.White,
    editor = Color.White,
    label = Color.Black,
    secondaryLabel = Color(0xFF8E8E93),
    tertiaryLabel = Color(0xFFC4C4C6),
    separator = Color(0xFFC6C6C8),
    fill = Color(0x1F767680),
    bar = Color(0xF2F6F6F6),
    alert = Color(0xFFF2F2F2),
    highlight = Color(0xFFD1D1D6),
    red = Color(0xFFFF3B30),
    green = Color(0xFF34C759),
    isDark = false,
)

private val DarkColors = IosColors(
    background = Color.Black,
    card = Color(0xFF1C1C1E),
    editor = Color.Black,
    label = Color.White,
    secondaryLabel = Color(0xFF8E8E93),
    tertiaryLabel = Color(0xFF5A5A5E),
    separator = Color(0xFF38383A),
    fill = Color(0x3D767680),
    bar = Color(0xF0161616),
    alert = Color(0xFF2C2C2E),
    highlight = Color(0xFF3A3A3C),
    red = Color(0xFFFF453A),
    green = Color(0xFF30D158),
    isDark = true,
)

/** iOS text sizes and weights, set in the system sans font. */
object IosType {
    val largeTitle = TextStyle(fontSize = 34.sp, lineHeight = 41.sp, fontWeight = FontWeight.Bold)
    val title2 = TextStyle(fontSize = 22.sp, lineHeight = 28.sp, fontWeight = FontWeight.Bold)
    val sectionHeader = TextStyle(fontSize = 20.sp, lineHeight = 25.sp, fontWeight = FontWeight.Bold)
    val headline = TextStyle(fontSize = 17.sp, lineHeight = 22.sp, fontWeight = FontWeight.SemiBold)
    val body = TextStyle(fontSize = 17.sp, lineHeight = 22.sp)
    val subhead = TextStyle(fontSize = 15.sp, lineHeight = 20.sp)
    val footnote = TextStyle(fontSize = 13.sp, lineHeight = 18.sp)
    val caption = TextStyle(fontSize = 12.sp, lineHeight = 16.sp)
    val noteTitle = TextStyle(fontSize = 28.sp, lineHeight = 34.sp, fontWeight = FontWeight.Bold)
}

val LocalIosColors = staticCompositionLocalOf { LightColors }

object NotesTheme {
    val colors: IosColors @Composable get() = LocalIosColors.current
}

@Composable
fun NotesTheme(dark: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
    val colors = if (dark) DarkColors else LightColors
    val scheme = if (dark) {
        darkColorScheme(
            primary = Accent, onPrimary = Color.Black, background = colors.background,
            surface = colors.card, surfaceContainer = colors.alert, onSurface = colors.label,
            surfaceContainerLow = colors.background,
        )
    } else {
        lightColorScheme(
            primary = Accent, onPrimary = Color.White, background = colors.background,
            surface = colors.card, surfaceContainer = colors.alert, onSurface = colors.label,
            surfaceContainerLow = colors.background,
        )
    }
    MaterialTheme(colorScheme = scheme) {
        CompositionLocalProvider(
            LocalIosColors provides colors,
            LocalIndication provides HighlightIndication(colors.highlight),
            LocalTextSelectionColors provides TextSelectionColors(Accent, Accent.copy(alpha = 0.35f)),
            content = content,
        )
    }
}

/** iOS-style press feedback: the row turns gray while pressed, no ripple. */
private class HighlightIndication(private val color: Color) : IndicationNodeFactory {
    override fun create(interactionSource: InteractionSource): DelegatableNode = HighlightNode(interactionSource, color)
    override fun equals(other: Any?) = other is HighlightIndication && other.color == color
    override fun hashCode() = color.hashCode()
}

private class HighlightNode(
    private val interactionSource: InteractionSource,
    private val color: Color,
) : Modifier.Node(), DrawModifierNode {
    private var pressed = false

    override fun onAttach() {
        coroutineScope.launch {
            var presses = 0
            interactionSource.interactions.collect {
                when (it) {
                    is PressInteraction.Press -> presses++
                    is PressInteraction.Release, is PressInteraction.Cancel -> presses--
                }
                if ((presses > 0) != pressed) {
                    pressed = presses > 0
                    invalidateDraw()
                }
            }
        }
    }

    override fun ContentDrawScope.draw() {
        if (pressed) drawRect(color)
        drawContent()
    }
}
