package io.github.enrell.lain.ui.theme

import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import io.github.enrell.lain.core.ThemeTokens
import kotlin.math.pow

/**
 * `lain.ui.theme@1` tokens as Compose values (D-249). Colors and sizes
 * come from the theme file; the font families map to the platform's
 * (the app bundles no fonts yet): a `mono` family to monospace, the
 * display family to serif, the rest to sans-serif.
 */
class Lain(val tokens: ThemeTokens) {
    private fun c(argb: Long) = Color(argb.toInt())

    val bg = c(tokens.colors.bg)
    val surface = c(tokens.colors.surface)
    val raised = c(tokens.colors.raised)
    val line = c(tokens.colors.line)
    val text = c(tokens.colors.text)
    val muted = c(tokens.colors.muted)
    val accent = c(tokens.colors.accent)
    val onAccent = c(tokens.colors.onAccent)
    val danger = c(tokens.colors.danger)
    val ok = c(tokens.colors.ok)
    val warn = c(tokens.colors.warn)
    val scrim = c(tokens.colors.scrim)
    val focus = c(tokens.colors.focus)
    val controlsBg = c(tokens.colors.controlsBg)
    val heroStops = tokens.colors.heroStops.map { c(it) }
    val dark = tokens.scheme != "light"

    val ui = family(tokens.fontUi)
    val display = if (tokens.fontDisplay.contains("mono", true)) FontFamily.Monospace else FontFamily.Serif
    val mono = FontFamily.Monospace

    val cardW: Dp = tokens.railCardW.dp * 0.6f
    val cardH: Dp = tokens.railCardH.dp * 0.6f
    val gap: Dp = tokens.railGap.dp * 0.75f
    val cardShape = RoundedCornerShape(tokens.radiusCard.dp)
    val buttonShape = RoundedCornerShape(tokens.radiusButton.dp)
    val panelShape = RoundedCornerShape(tokens.radiusPanel.dp)
    val focusScale: Float = tokens.railHoverScale
    val easing: Easing = tokens.easing.let { e -> if (e.size == 4) CubicBezierEasing(e[0], e[1], e[2], e[3]) else CubicBezierEasing(0.33f, 1f, 0.68f, 1f) }
    val fastMs: Int = tokens.motionFastMs.toInt()
    val normalMs: Int = tokens.motionNormalMs.toInt()

    /** `spacing[i]`, clamped to the scale. */
    fun sp(i: Int): Dp = (tokens.spacing.getOrNull(i) ?: tokens.spacing.lastOrNull() ?: 8f).dp

    /** Type step: 0 is the base, labels are −2, titles 2..5. */
    fun ts(step: Int) = (tokens.typeBase * tokens.typeScale.toDouble().pow(step).toFloat()).sp

    /** Mono uppercase labels (D-022). */
    fun label(color: Color = muted) = TextStyle(fontFamily = mono, fontSize = ts(-2), letterSpacing = 0.08.em, color = color)

    private fun family(name: String): FontFamily = when {
        name.contains("mono", true) -> FontFamily.Monospace
        name.contains("serif", true) && !name.contains("sans", true) -> FontFamily.Serif
        else -> FontFamily.SansSerif
    }

    fun colorScheme(): ColorScheme {
        val base = if (dark) darkColorScheme() else lightColorScheme()
        return base.copy(
            primary = accent,
            onPrimary = onAccent,
            secondary = accent,
            onSecondary = onAccent,
            background = bg,
            onBackground = text,
            surface = bg,
            onSurface = text,
            surfaceVariant = surface,
            onSurfaceVariant = muted,
            surfaceContainer = raised,
            surfaceContainerHigh = raised,
            surfaceContainerHighest = surface,
            outline = line,
            outlineVariant = line,
            error = danger,
            onError = onAccent,
            scrim = scrim,
        )
    }

    fun typography(): Typography {
        val t = Typography()
        fun s(step: Int, weight: FontWeight = FontWeight.Normal, f: FontFamily = ui) =
            TextStyle(fontFamily = f, fontSize = ts(step), fontWeight = weight, color = text)
        return t.copy(
            displayLarge = s(5, FontWeight.SemiBold, display).copy(letterSpacing = (-0.02).em),
            displayMedium = s(4, FontWeight.SemiBold, display).copy(letterSpacing = (-0.02).em),
            headlineLarge = s(3, FontWeight.SemiBold, display),
            headlineMedium = s(2, FontWeight.SemiBold, display),
            titleLarge = s(2, FontWeight.Medium),
            titleMedium = s(1, FontWeight.Medium),
            titleSmall = s(0, FontWeight.Medium),
            bodyLarge = s(1),
            bodyMedium = s(0),
            bodySmall = s(-1),
            labelLarge = s(0, FontWeight.Medium),
            labelMedium = s(-1, FontWeight.Medium, mono),
            labelSmall = s(-2, FontWeight.Medium, mono),
        )
    }

    fun shapes() = Shapes(
        small = buttonShape,
        medium = cardShape,
        large = panelShape,
    )
}

val LocalLain = staticCompositionLocalOf<Lain> { error("LainTheme missing") }

@Composable
fun LainTheme(tokens: ThemeTokens, content: @Composable () -> Unit) {
    val lain = remember(tokens) { Lain(tokens) }
    CompositionLocalProvider(LocalLain provides lain) {
        MaterialTheme(colorScheme = lain.colorScheme(), typography = lain.typography(), shapes = lain.shapes(), content = content)
    }
}
