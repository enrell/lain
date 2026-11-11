package io.github.enrell.lain.ui.screens

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.enrell.lain.R
import io.github.enrell.lain.core.LainException
import io.github.enrell.lain.graph
import io.github.enrell.lain.ui.theme.LocalLain

/** A key hint shown next to its action (AGENTS.md: every action has a key). */
@Composable
fun Kbd(key: String, modifier: Modifier = Modifier) {
    val l = LocalLain.current
    Text(
        key,
        style = l.label(l.muted),
        modifier = modifier
            .border(1.dp, l.line, l.buttonShape)
            .padding(horizontal = 5.dp, vertical = 1.dp),
    )
}

/** A mono uppercase label (D-022). */
@Composable
fun Label(text: String, modifier: Modifier = Modifier, color: Color? = null) {
    val l = LocalLain.current
    Text(text.uppercase(), style = l.label(color ?: l.muted), modifier = modifier)
}

/** A focusable action: Enter or a tap runs it; its key is shown. */
@Composable
fun Action(text: String, key: String?, onClick: () -> Unit, modifier: Modifier = Modifier, primary: Boolean = false, enabled: Boolean = true) {
    val l = LocalLain.current
    val source = remember { MutableInteractionSource() }
    val focused by source.collectIsFocusedAsState()
    Row(
        modifier = modifier
            .clip(l.buttonShape)
            .background(if (primary) l.accent else l.raised)
            .border(if (focused) 2.dp else 1.dp, if (focused) l.focus else l.line, l.buttonShape)
            .clickable(enabled = enabled, interactionSource = source, indication = null, onClick = onClick)
            .padding(horizontal = l.sp(3), vertical = l.sp(1)),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(l.sp(1)),
    ) {
        Text(text, style = MaterialTheme.typography.labelLarge, color = if (primary) l.onAccent else if (enabled) l.text else l.muted)
        if (key != null) Kbd(key)
    }
}

/** A human reason for a failure, translated; the server's own detail is shown as is. */
@Composable
fun errorText(e: Throwable): String = when (e) {
    is LainException.Offline, is LainException.Timeout -> stringResource(R.string.err_offline)
    is LainException.Unauthorized -> stringResource(R.string.err_unauthorized)
    is LainException.LoginFailed -> stringResource(R.string.err_login, e.detail)
    is LainException.Forbidden -> e.detail.ifBlank { stringResource(R.string.err_forbidden) }
    is LainException.NotFound -> e.detail.ifBlank { stringResource(R.string.err_not_found) }
    is LainException.ModuleDisabled -> stringResource(R.string.err_module_disabled, e.module)
    is LainException.NotPorted -> stringResource(R.string.err_not_ported)
    is LainException.NoRoom -> stringResource(R.string.err_no_room, e.detail)
    is LainException.Unplayable -> e.detail
    is LainException.Unsupported -> e.detail
    is LainException.Invalid -> e.detail
    is LainException.Api -> e.detail.ifBlank { stringResource(R.string.err_server, e.status.toInt()) }
    is LainException.NoServer -> stringResource(R.string.err_no_server)
    is LainException -> e.message ?: e.toString()
    else -> e.message ?: e.toString()
}

@Composable
fun ErrorLine(e: Throwable?, modifier: Modifier = Modifier) {
    if (e == null) return
    val l = LocalLain.current
    Text(errorText(e), color = l.danger, style = MaterialTheme.typography.bodyMedium, modifier = modifier)
}

/** An item's art through the shared cache; a surface tile while loading or without art. */
@Composable
fun Art(itemId: String, modifier: Modifier = Modifier, scale: ContentScale = ContentScale.Crop) {
    val l = LocalLain.current
    val ctx = LocalContext.current
    // What memory already holds paints in the first frame.
    val bmp: ImageBitmap? by produceState(ctx.graph.artNow(itemId)?.also { Shapes.record(itemId, it.width, it.height) }, itemId) {
        if (value == null) value = ctx.graph.art(itemId)?.also { Shapes.record(itemId, it.width, it.height) }
    }
    Box(modifier.background(l.surface)) {
        bmp?.let { Image(it, contentDescription = null, contentScale = scale, modifier = Modifier.fillMaxSize()) }
    }
}

/** A poster card: art, title, a progress line; it grows when focused (theme `rail.hover_scale`). */
@Composable
fun PosterCard(
    artId: String,
    title: String,
    subtitle: String,
    progress: Float,
    onClick: () -> Unit,
    width: Dp? = null,
    height: Dp? = null,
) {
    val l = LocalLain.current
    val source = remember { MutableInteractionSource() }
    val focused by source.collectIsFocusedAsState()
    val grow by animateFloatAsState(if (focused) l.focusScale else 1f, tween(l.fastMs, easing = l.easing), label = "focus")
    val w = width ?: l.cardW
    Column(
        Modifier
            .width(w)
            .scale(grow)
            .clickable(interactionSource = source, indication = null, onClick = onClick),
    ) {
        Box(
            Modifier
                .width(w)
                .height(height ?: l.cardH)
                .clip(l.cardShape)
                .border(if (focused) 2.dp else 0.dp, if (focused) l.focus else Color.Transparent, l.cardShape),
        ) {
            Art(artId, Modifier.fillMaxSize())
            if (progress > 0f) {
                LinearProgressIndicator(
                    progress = { progress },
                    color = l.accent,
                    trackColor = l.scrim,
                    modifier = Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(3.dp),
                )
            }
        }
        Text(title, style = MaterialTheme.typography.titleSmall, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = l.sp(1)))
        if (subtitle.isNotEmpty()) Label(subtitle)
    }
}

/** A full-bleed scrim from the theme's hero stops over art. */
fun heroScrim(stops: List<Color>, bg: Color): Brush =
    Brush.verticalGradient(if (stops.size >= 2) stops else listOf(Color.Transparent, bg))

@Composable
fun Centered(content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { content() }
}

@Composable
fun Dot(color: Color) {
    Box(Modifier.size(8.dp).clip(androidx.compose.foundation.shape.CircleShape).background(color).focusable(false))
}

/** `S01E02`, `E02`, `Vol. 3`, or empty. */
fun episodeLabel(season: Long, episode: Long, reading: Boolean): String = when {
    reading && episode > 0 -> "#$episode"
    season > 0 && episode > 0 -> "S%02dE%02d".format(season, episode)
    episode > 0 -> "E%02d".format(episode)
    else -> ""
}

fun bytes(n: Long): String {
    val units = listOf("B", "KiB", "MiB", "GiB", "TiB")
    var v = n.toDouble()
    var i = 0
    while (v >= 1024 && i < units.lastIndex) {
        v /= 1024
        i++
    }
    return if (i == 0) "$n B" else "%.1f %s".format(v, units[i])
}
