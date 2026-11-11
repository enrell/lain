package io.github.enrell.lain.ui.screens

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.ui.unit.dp

/** The web client's icons (Lucide, 24 px grid, 2 px round strokes). */
object Ic {
    private fun stroke(vararg d: String, filled: Boolean = false): ImageVector {
        val b = ImageVector.Builder(defaultWidth = 24.dp, defaultHeight = 24.dp, viewportWidth = 24f, viewportHeight = 24f)
        for (path in d) {
            b.addPath(
                pathData = addPathNodes(path),
                fill = if (filled) SolidColor(Color.Black) else null,
                stroke = SolidColor(Color.Black),
                strokeLineWidth = 2f,
                strokeLineCap = StrokeCap.Round,
                strokeLineJoin = StrokeJoin.Round,
            )
        }
        return b.build()
    }

    val Home by lazy {
        stroke(
            "M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8",
            "M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
        )
    }
    val Library by lazy { stroke("m16 6 4 14", "M12 6v14", "M8 8v12", "M4 4v16") }
    val Search by lazy { stroke("M11 3a8 8 0 1 0 0 16a8 8 0 1 0 0-16", "m21 21-4.34-4.34") }
    val Download by lazy { stroke("M12 15V3", "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4", "m7 10 5 5 5-5") }
    val More by lazy {
        stroke("M12 11a1 1 0 1 0 0 2a1 1 0 1 0 0-2", "M19 11a1 1 0 1 0 0 2a1 1 0 1 0 0-2", "M5 11a1 1 0 1 0 0 2a1 1 0 1 0 0-2", filled = true)
    }
    val Sliders by lazy { stroke("M21 4h-7", "M10 4H3", "M21 12h-9", "M8 12H3", "M21 20h-5", "M12 20H3", "M14 2v4", "M8 10v4", "M16 18v4") }
    val Book by lazy {
        stroke(
            "M12 7v14",
            "M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z",
        )
    }
    val Play by lazy { stroke("M6 4l14 8-14 8z", filled = true) }
    val Pause by lazy { stroke("M7 4v16", "M17 4v16") }
    val ChevronRight by lazy { stroke("m9 18 6-6-6-6") }
    val ChevronLeft by lazy { stroke("m15 18-6-6 6-6") }
    val ArrowLeft by lazy { stroke("m12 19-7-7 7-7", "M19 12H5") }
    val ArrowUp by lazy { stroke("m5 12 7-7 7 7", "M12 19V5") }
    val Close by lazy { stroke("M18 6 6 18", "m6 6 12 12") }
    val Info by lazy { stroke("M12 2a10 10 0 1 0 0 20a10 10 0 1 0 0-20", "M12 16v-4", "M12 8h.01") }
    val Check by lazy { stroke("M20 6 9 17l-5-5") }
}
