package io.github.enrell.lain.ui.screens

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.coerceAtLeast
import androidx.compose.ui.unit.coerceIn
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.media3.exoplayer.ExoPlayer
import io.github.enrell.lain.R
import io.github.enrell.lain.core.Item
import io.github.enrell.lain.graph
import io.github.enrell.lain.ui.PlayerModel
import io.github.enrell.lain.ui.PlayerState
import io.github.enrell.lain.ui.theme.LocalLain
import kotlinx.coroutines.delay

/** Footer: the seek row, then the transport. */
@Composable
internal fun BoxScope.PlayerBar(
    ui: PlayerUi,
    moves: PlayerMoves,
    model: PlayerModel,
    state: PlayerState,
    itemId: String,
    player: ExoPlayer,
    next: Item?,
    onNext: (Item) -> Unit,
    canPip: Boolean,
    onPip: () -> Unit,
) {
    val total = moves.totalSec().coerceAtLeast(0.001)
    val at = ui.scrub?.let { it * total } ?: moves.nowSec()
    Column(
        Modifier.align(Alignment.BottomStart).fillMaxWidth()
            .background(Brush.verticalGradient(0f to Color.Transparent, 0.45f to Color.Black.copy(alpha = 0.6f), 1f to Color.Black.copy(alpha = 0.88f))).safeDrawingPadding().padding(horizontal = 12.dp).padding(bottom = 10.dp, top = 36.dp),
    ) {
        SeekRow(ui, moves, model, state, itemId, total, at)
        BarActions(ui, player, next, onNext, canPip, onPip)
    }
}

/** The seek line and the time beside it. */
@Composable
private fun SeekRow(ui: PlayerUi, moves: PlayerMoves, model: PlayerModel, state: PlayerState, itemId: String, total: Double, at: Double) {
    val l = LocalLain.current
    Row(verticalAlignment = Alignment.CenterVertically) {
        BoxWithConstraints(Modifier.weight(1f)) {
            val width = maxWidth
            val fraction = (at / total).toFloat().coerceIn(0f, 1f)
            // The frame the bar points at, while dragging it.
            ui.scrub?.let { f -> ScrubPreview(ui, itemId, f, total, width) }
            SeekLine(ui, moves, model, state, total, width, fraction)
        }
        Text(
            if (ui.showRemaining) "−" + clock((moves.totalSec() - at).coerceAtLeast(0.0)) else clock(at) + " / " + clock(moves.totalSec()),
            Modifier.padding(start = 6.dp).clip(l.buttonShape).clickable { ui.showRemaining = !ui.showRemaining; ui.touched++ }.padding(horizontal = 6.dp, vertical = 8.dp),
            color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.Medium,
        )
    }
}

/** The frame at [f] of the bar ([width] wide), on a card above it with its time. */
@Composable
private fun ScrubPreview(ui: PlayerUi, itemId: String, f: Float, total: Double, width: Dp) {
    val graph = LocalContext.current.graph
    val sec = f * total
    val sheet = ui.board
    if (sheet == null) {
        // No storyboard (yet): single frames on a coarse grid,
        // one request at a time, always for where the finger is.
        val step = (total / 150).toLong().coerceAtLeast(2)
        val quantized = (sec.toLong() / step) * step
        val wanted by rememberUpdatedState(quantized)
        LaunchedEffect(Unit) {
            var last = -1L
            while (true) {
                val want = wanted
                if (want != last) {
                    last = want
                    graph.frame(itemId, want)?.let { ui.preview = it }
                } else {
                    delay(40)
                }
            }
        }
    }
    val w = 176.dp
    val x = (width * f - w / 2).coerceIn(0.dp, (width - w).coerceAtLeast(0.dp))
    Column(
        // Floats above the bar without taking room in the row.
        // (The layout comes first: a node that reports a size
        // under its minimum is centred by its parent.)
        Modifier.layout { measurable, constraints ->
            val p = measurable.measure(constraints.copy(minWidth = 0, minHeight = 0))
            layout(0, 0) { p.place(x.roundToPx(), -p.height - 6.dp.roundToPx()) }
        }.width(w).clip(RoundedCornerShape(10.dp)).background(Color.Black.copy(alpha = 0.85f)).padding(4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.fillMaxWidth().aspectRatio(16f / 9f).clip(RoundedCornerShape(8.dp)).background(Color.Black).border(1.dp, Color.White.copy(alpha = 0.2f), RoundedCornerShape(8.dp))) {
            if (sheet != null) {
                // The tile for this second, cut out of the sheet: nothing to fetch.
                val (x, y) = sheet.tileAt(sec)
                Canvas(Modifier.fillMaxSize()) {
                    drawImage(
                        sheet.image,
                        srcOffset = IntOffset(x, y),
                        srcSize = IntSize(sheet.tileWidth, sheet.tileHeight),
                        dstSize = IntSize(size.width.toInt(), size.height.toInt()),
                    )
                }
            } else {
                ui.preview?.let { Image(it, null, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize()) }
            }
        }
        // Its time under it, on the same card: off the picture, and not floating.
        Text(clock(sec), Modifier.padding(top = 5.dp, bottom = 1.dp), color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
    }
}

/** The line itself: what is loaded, where it is, and a tap or a drag along it to seek. */
@Composable
private fun SeekLine(ui: PlayerUi, moves: PlayerMoves, model: PlayerModel, state: PlayerState, total: Double, width: Dp, fraction: Float) {
    val l = LocalLain.current
    Box(
        Modifier.fillMaxWidth().height(36.dp)
            .pointerInput(Unit) {
                detectTapGestures { o ->
                    if (moves.totalSec() > 0) moves.seekToSec(o.x / size.width * moves.totalSec())
                    ui.touched++
                }
            }
            .pointerInput(Unit) {
                detectHorizontalDragGestures(
                    onDragStart = { o -> ui.settings = false; ui.tracksPanel = 0; if (moves.totalSec() > 0) ui.scrub = (o.x / size.width).coerceIn(0f, 1f) },
                    onDragEnd = {
                        ui.scrub?.let { moves.seekToSec(it * moves.totalSec()) }
                        ui.scrub = null
                        ui.touched++
                    },
                    onDragCancel = { ui.scrub = null },
                ) { change, _ -> if (moves.totalSec() > 0) ui.scrub = (change.position.x / size.width).coerceIn(0f, 1f) }
            },
        contentAlignment = Alignment.CenterStart,
    ) {
        Box(Modifier.fillMaxWidth().height(5.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.25f)))
        // What is loaded sits where it is: a session that began
        // partway holds a range inside the title, not its start.
        val heldFrom = ((state.source?.offsetSec ?: 0.0) / total).toFloat().coerceIn(0f, 1f)
        val heldTo = (model.sourceSec(ui.bufferedMs) / total).toFloat().coerceIn(heldFrom, 1f)
        Box(Modifier.offset(x = width * heldFrom).width(width * (heldTo - heldFrom)).height(5.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.3f)))
        Box(Modifier.fillMaxWidth(fraction).height(5.dp).clip(CircleShape).background(l.accent))
        Box(Modifier.offset(x = (width - 18.dp) * fraction).size(if (ui.scrub != null) 24.dp else 18.dp).clip(CircleShape).background(l.accent))
    }
}

/** The bar's actions under the seek row: next, speed, mute, the track lists, settings, the corner and fullscreen. */
@Composable
private fun BarActions(ui: PlayerUi, player: ExoPlayer, next: Item?, onNext: (Item) -> Unit, canPip: Boolean, onPip: () -> Unit) {
    val l = LocalLain.current
    val activity = LocalContext.current as? android.app.Activity
    Row(Modifier.padding(top = 2.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        if (next != null) IconBtn(Ic.SkipNext, stringResource(R.string.player_next), { onNext(next) }, tint = Color.White)
        if (ui.speed != 1f) {
            Text(
                "${ui.speed}×",
                Modifier.padding(start = 4.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.16f)).clickable { ui.tracksPanel = 0; ui.settings = true }.padding(horizontal = 10.dp, vertical = 5.dp),
                color = Color.White, fontSize = 12.sp, fontWeight = FontWeight.SemiBold,
            )
        }
        IconBtn(if (ui.muted) Ic.Muted else Ic.Volume, "Mute", { ui.muted = !ui.muted; player.volume = if (ui.muted) 0f else 1f; ui.touched++ }, tint = Color.White)
        Box(Modifier.weight(1f))
        // Subtitles and audio are on the bar, not behind the gear.
        // Named, not only drawn: a waveform alone does not say "audio track".
        if (ui.textTracks.isNotEmpty()) {
            val on = ui.tracksPanel == 1 || (!ui.textOff && ui.textTracks.any { it.selected })
            // The button names what is on: "Subtitles · English", or just "Subtitles" when they are off.
            val shown = ui.textTracks.firstOrNull { it.selected }?.takeIf { !ui.textOff }?.label?.substringBefore(" · ")
            BarAction(Ic.Captions, stringResource(R.string.player_subtitles) + (shown?.let { " · $it" } ?: ""), on) { ui.settings = false; ui.tracksPanel = if (ui.tracksPanel == 1) 0 else 1 }
        }
        if (ui.audioTracks.size > 1) {
            val heard = ui.audioTracks.firstOrNull { it.selected }?.label?.substringBefore(" · ")
            // Always at full weight: some track always plays, and the dimmed
            // "off" look read as a button that could not be used.
            BarAction(Ic.Audio, stringResource(R.string.player_audio) + (heard?.let { " · $it" } ?: ""), on = true) { ui.settings = false; ui.tracksPanel = if (ui.tracksPanel == 2) 0 else 2 }
        }
        IconBtn(Ic.Gear, "Settings", { ui.tracksPanel = 0; ui.settings = !ui.settings }, tint = if (ui.settings) l.accent else Color.White)
        if (canPip) IconBtn(Ic.Pip, stringResource(R.string.player_pip), { onPip() }, tint = Color.White)
        IconBtn(Ic.Maximize, "Fullscreen", {
            ui.landscape = !ui.landscape
            activity?.requestedOrientation =
                if (ui.landscape) android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE else restOrientation(activity)
            ui.touched++
        }, tint = Color.White)
    }
}

/** An action of the player's bar that needs its name beside its drawing. */
@Composable
private fun BarAction(icon: ImageVector, label: String, on: Boolean, onClick: () -> Unit) {
    val l = LocalLain.current
    Row(
        Modifier.height(44.dp).clip(CircleShape).clickable(onClickLabel = label, onClick = onClick).padding(horizontal = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        // On is the bright one (white, bold); off steps back. A rose tint on dark video read as dimmed.
        Icon(icon, null, tint = if (on) Color.White else Color.White.copy(alpha = 0.7f), modifier = Modifier.size(20.dp))
        Text(label, color = if (on) Color.White else Color.White.copy(alpha = 0.7f), fontSize = 13.sp, fontWeight = if (on) FontWeight.SemiBold else FontWeight.Medium, maxLines = 1)
    }
}
