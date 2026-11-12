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
class Lain(val tokens: ThemeTokens, val web: WebPalette = WebPalette.Default) {
    private fun c(argb: Long) = Color(argb.toInt())

    // Colours are the server's (the web client's); the theme file keeps
    // spacing, motion and the player's values.
    val bg = web.background
    val surface = web.surface
    val raised = web.surfaceActive
    val hover = web.surfaceHover
    val line = web.line
    val text = web.foreground
    val muted = web.muted
    val accent = web.accent
    val onAccent = web.accentForeground
    val danger = web.danger
    val ok = web.success
    val warn = web.warning
    /** The foreground at [f] over the background, opaque. */
    private fun lift(f: Float) = Color(
        red = web.background.red + (web.foreground.red - web.background.red) * f,
        green = web.background.green + (web.foreground.green - web.background.green) * f,
        blue = web.background.blue + (web.foreground.blue - web.background.blue) * f,
    )
    /** A field's fill: the foreground at 6 % over the background, as on the web. */
    val field = lift(0.06f)
    /**
     * A sheet's ground, one step above the page. A theme's `surface` is often
     * the background itself, and a sheet on it read as a hole in the scrim,
     * not as something raised.
     */
    val sheet = lift(0.045f)
    /**
     * A control's or a row's ground: the foreground at 8 % and 5 % over the
     * page. Surfaces are told apart by fill, not by hairlines: outlines on
     * near-black made every screen read as a wireframe.
     */
    val tonal = web.foreground.copy(alpha = 0.08f)
    val tonalLow = web.foreground.copy(alpha = 0.05f)
    val scrim = web.background.copy(alpha = 0.8f)
    val focus = web.accent
    val controlsBg = c(tokens.colors.controlsBg)
    val heroStops = listOf(Color.Transparent, web.background.copy(alpha = 0.55f), web.background)
    val dark = web.dark

    val ui = family(tokens.fontUi)
    val display = FontFamily.SansSerif
    val mono = FontFamily.Monospace

    val cardW: Dp = tokens.railCardW.dp * 0.6f
    val cardH: Dp = tokens.railCardH.dp * 0.6f
    val gap: Dp = tokens.railGap.dp * 0.75f
    val cardShape = RoundedCornerShape(12.dp)
    val buttonShape = RoundedCornerShape(6.dp)
    val panelShape = RoundedCornerShape(12.dp)
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
fun LainTheme(tokens: ThemeTokens, web: WebPalette = WebPalette.Default, content: @Composable () -> Unit) {
    val lain = remember(tokens, web) { Lain(tokens, web) }
    CompositionLocalProvider(LocalLain provides lain) {
        MaterialTheme(colorScheme = lain.colorScheme(), typography = lain.typography(), shapes = lain.shapes(), content = content)
    }
}
