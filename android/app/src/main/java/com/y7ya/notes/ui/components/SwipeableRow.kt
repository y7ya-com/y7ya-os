package com.y7ya.notes.ui.components

import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.Orientation
import androidx.compose.foundation.gestures.draggable
import androidx.compose.foundation.gestures.rememberDraggableState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlinx.coroutines.launch

data class SwipeAction(
    val icon: ImageVector,
    val label: String,
    val color: Color,
    val onAction: () -> Unit,
)

private val ActionWidth = 76.dp

/**
 * A list row with iOS swipe actions: drag to reveal a button, or swipe all the way
 * to trigger it. Only the row whose key equals [openKey] stays open.
 */
@Composable
fun SwipeableRow(
    key: Any,
    openKey: Any?,
    onOpenKeyChange: (Any?) -> Unit,
    modifier: Modifier = Modifier,
    leading: SwipeAction? = null,
    trailing: SwipeAction? = null,
    content: @Composable () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val offset = remember { Animatable(0f) }
    val actionWidth = with(LocalDensity.current) { ActionWidth.toPx() }

    LaunchedEffect(openKey) {
        if (openKey != key && offset.value != 0f) offset.animateTo(0f, spring())
    }

    BoxWithConstraints(modifier) {
        val width = constraints.maxWidth.toFloat()
        val fullSwipe = width * 0.6f

        fun settle(velocity: Float) {
            val x = offset.value
            scope.launch {
                when {
                    trailing != null && (x < -fullSwipe || (x < -actionWidth && velocity < -3000f)) -> {
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        offset.animateTo(-width)
                        trailing.onAction()
                        onOpenKeyChange(null)
                        offset.snapTo(0f)
                    }
                    leading != null && (x > fullSwipe || (x > actionWidth && velocity > 3000f)) -> {
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        offset.animateTo(0f, spring())
                        leading.onAction()
                        onOpenKeyChange(null)
                    }
                    x < -actionWidth / 2 && trailing != null -> offset.animateTo(-actionWidth, spring())
                    x > actionWidth / 2 && leading != null -> offset.animateTo(actionWidth, spring())
                    else -> {
                        offset.animateTo(0f, spring())
                        if (openKey == key) onOpenKeyChange(null)
                    }
                }
            }
        }

        val dragState = rememberDraggableState { delta ->
            val min = if (trailing != null) -width else 0f
            val max = if (leading != null) width else 0f
            scope.launch { offset.snapTo((offset.value + delta).coerceIn(min, max)) }
        }

        // Action buttons revealed behind the row.
        val x = offset.value
        Box(Modifier.matchParentSize()) {
            if (x < 0 && trailing != null) {
                ActionButton(trailing, abs(x), Modifier.align(Alignment.CenterEnd)) {
                    scope.launch { offset.animateTo(0f, spring()) }
                    onOpenKeyChange(null)
                    trailing.onAction()
                }
            }
            if (x > 0 && leading != null) {
                ActionButton(leading, x, Modifier.align(Alignment.CenterStart)) {
                    scope.launch { offset.animateTo(0f, spring()) }
                    onOpenKeyChange(null)
                    leading.onAction()
                }
            }
        }

        Box(
            Modifier
                .offset { IntOffset(offset.value.roundToInt(), 0) }
                .draggable(
                    dragState,
                    Orientation.Horizontal,
                    enabled = leading != null || trailing != null,
                    onDragStarted = { onOpenKeyChange(key) },
                    onDragStopped = { velocity -> settle(velocity) },
                ),
        ) {
            content()
            if (offset.value != 0f) {
                // Tapping an open row closes it instead of opening the note.
                Box(
                    Modifier.matchParentSize().clickable(
                        interactionSource = null,
                        indication = null,
                    ) {
                        scope.launch { offset.animateTo(0f, spring()) }
                        onOpenKeyChange(null)
                    },
                )
            }
        }
    }
}

@Composable
private fun ActionButton(action: SwipeAction, widthPx: Float, modifier: Modifier, onClick: () -> Unit) {
    val width = with(LocalDensity.current) { widthPx.toDp() }
    Box(
        modifier
            .width(width)
            .fillMaxHeight()
            .background(action.color)
            .clickable(onClickLabel = action.label, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(action.icon, contentDescription = action.label, tint = Color.White, modifier = Modifier.size(24.dp))
    }
}
