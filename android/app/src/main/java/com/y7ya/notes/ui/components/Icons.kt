package com.y7ya.notes.ui.components

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/** Line icons in the style of SF Symbols, drawn on a 24×24 grid. Tint them with `Icon`. */
object IosIcons {
    val ChevronBack = stroke("chevron_back", "M15.5,4.5 L8,12 L15.5,19.5", width = 2.6f)
    val ChevronForward = stroke("chevron_forward", "M9,5.5 L15.5,12 L9,18.5", width = 2.4f)

    val Folder = stroke(
        "folder",
        "M3,7.2 A2.2,2.2 0 0 1 5.2,5 H9.3 L11.3,7.2 H18.8 A2.2,2.2 0 0 1 21,9.4 V17 A2.2,2.2 0 0 1 18.8,19.2 " +
            "H5.2 A2.2,2.2 0 0 1 3,17 Z M3,9.6 H21",
    )

    val NewFolder = stroke(
        "folder_badge_plus",
        "M3,7.2 A2.2,2.2 0 0 1 5.2,5 H9.3 L11.3,7.2 H18.8 A2.2,2.2 0 0 1 21,9.4 V17 A2.2,2.2 0 0 1 18.8,19.2 " +
            "H5.2 A2.2,2.2 0 0 1 3,17 Z M12,10.6 V16.6 M9,13.6 H15",
    )

    val Trash = stroke(
        "trash",
        "M3.8,6.5 H20.2 M9,6.5 V4.6 A0.9,0.9 0 0 1 9.9,3.7 H14.1 A0.9,0.9 0 0 1 15,4.6 V6.5 " +
            "M5.9,6.5 L6.9,19.1 A1.7,1.7 0 0 0 8.6,20.6 H15.4 A1.7,1.7 0 0 0 17.1,19.1 L18.1,6.5 " +
            "M10,10.2 V16.9 M14,10.2 V16.9",
    )

    val Compose = stroke(
        "square_and_pencil",
        "M11.5,4.6 H6.6 A2.1,2.1 0 0 0 4.5,6.7 V17.4 A2.1,2.1 0 0 0 6.6,19.5 H17.3 A2.1,2.1 0 0 0 19.4,17.4 V12.5 " +
            "M17.7,3.7 A1.55,1.55 0 0 1 19.9,5.9 L12.2,13.6 L9.2,14.5 L10.1,11.4 Z",
    )

    val Pin = stroke(
        "pin",
        "M8.5,3.5 H15.5 M10,3.5 V9.2 L7,13.2 H17 L14,9.2 V3.5 M12,13.2 V20.5",
    )

    val Unpin = stroke(
        "pin_slash",
        "M8.5,3.5 H15.5 M10,3.5 V9.2 L7,13.2 H17 L14,9.2 V3.5 M12,13.2 V20.5 M4,4 L20,20",
    )

    val Recover = stroke(
        "arrow_uturn_backward",
        "M8.5,13.5 L4,9 L8.5,4.5 M4,9 H14.5 A5,5 0 0 1 14.5,19 H10.5",
    )

    val Search = stroke("magnifyingglass", "M4.5,10.5 A6,6 0 1 0 16.5,10.5 A6,6 0 1 0 4.5,10.5 Z M15,15 L20,20", width = 2.1f)

    val Computer = stroke(
        "laptopcomputer",
        "M5.5,6.2 A1.2,1.2 0 0 1 6.7,5 H17.3 A1.2,1.2 0 0 1 18.5,6.2 V15.5 H5.5 Z M2.5,18.5 H21.5",
    )

    val Scan = stroke(
        "qrcode_viewfinder",
        "M3.5,8.5 V5.5 A2,2 0 0 1 5.5,3.5 H8.5 M15.5,3.5 H18.5 A2,2 0 0 1 20.5,5.5 V8.5 " +
            "M20.5,15.5 V18.5 A2,2 0 0 1 18.5,20.5 H15.5 M8.5,20.5 H5.5 A2,2 0 0 1 3.5,18.5 V15.5 " +
            "M8,8 H11 V11 H8 Z M13,8 H16 V11 H13 Z M8,13 H11 V16 H8 Z M13.5,13.5 H15.5 V15.5 H13.5 Z",
        width = 1.6f,
    )

    val Ellipsis = ImageVector.Builder("ellipsis_circle", 24.dp, 24.dp, 24f, 24f).apply {
        addPath(addPathNodes("M3,12 A9,9 0 1 0 21,12 A9,9 0 1 0 3,12 Z"), stroke = SolidColor(Color.Black), strokeLineWidth = 1.7f)
        addPath(addPathNodes(dots(7.5f) + dots(12f) + dots(16.5f)), fill = SolidColor(Color.Black))
    }.build()

    private fun dots(cx: Float) = "M${cx - 1.3f},12 A1.3,1.3 0 1 0 ${cx + 1.3f},12 A1.3,1.3 0 1 0 ${cx - 1.3f},12 Z "

    private fun stroke(name: String, pathData: String, width: Float = 1.8f): ImageVector =
        ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).apply {
            addPath(
                pathData = addPathNodes(pathData),
                stroke = SolidColor(Color.Black),
                strokeLineWidth = width,
                strokeLineCap = StrokeCap.Round,
                strokeLineJoin = StrokeJoin.Round,
            )
        }.build()
}
