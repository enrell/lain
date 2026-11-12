package io.github.enrell.lain.ui.screens

import android.view.ViewGroup
import androidx.annotation.OptIn
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.MutableState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import io.github.enrell.lain.R
import io.github.enrell.lain.ui.PlayerState
import io.github.enrell.lain.ui.theme.LocalLain
import kotlinx.coroutines.delay

/** The picture: Media3's view without its own controls, subtitles placed and styled for the chrome. */
@OptIn(UnstableApi::class)
@Composable
internal fun PlayerPicture(player: ExoPlayer, ui: PlayerUi, pinned: Boolean) {
    AndroidView(
        factory = { ctx ->
            PlayerView(ctx).apply {
                layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
                this.player = player
                // The chrome is the app's own, drawn like the web player's.
                useController = false
            }
        },
        // Subtitles sit above the bar while it is up, and near the edge when it is not.
        update = { view ->
            // The next episode comes with a new player while this view stays: hand it over,
            // or the picture stays on the old, released one.
            if (view.player !== player) view.player = player
            view.subtitleView?.setBottomPaddingFraction(if (ui.controls || pinned) 0.24f else 0.08f)
            // While the bar is dragged its preview card stands where the subtitles are, and
            // the line showed through under it; they return when the finger lifts.
            view.subtitleView?.visibility = if (ui.scrub != null) android.view.View.INVISIBLE else android.view.View.VISIBLE
            // The size and ground chosen in Settings.
            view.subtitleView?.setFractionalTextSize(androidx.media3.ui.SubtitleView.DEFAULT_TEXT_SIZE_FRACTION * ui.subtitleScale)
            view.subtitleView?.setStyle(
                androidx.media3.ui.CaptionStyleCompat(
                    android.graphics.Color.WHITE,
                    if (ui.subtitleBox) android.graphics.Color.argb(170, 0, 0, 0) else android.graphics.Color.TRANSPARENT,
                    android.graphics.Color.TRANSPARENT,
                    if (ui.subtitleBox) androidx.media3.ui.CaptionStyleCompat.EDGE_TYPE_NONE else androidx.media3.ui.CaptionStyleCompat.EDGE_TYPE_OUTLINE,
                    android.graphics.Color.BLACK,
                    null,
                ),
            )
            // Fill crops the picture to the screen; Fit shows it whole with bars.
            view.resizeMode = if (ui.fill) androidx.media3.ui.AspectRatioFrameLayout.RESIZE_MODE_ZOOM else androidx.media3.ui.AspectRatioFrameLayout.RESIZE_MODE_FIT
        },
        modifier = Modifier.fillMaxSize(),
    )
}

/**
 * The gestures over the picture: a drag seeks, a double tap on the left or
 * right third skips (and keeps counting while the taps continue), a double
 * tap in the middle plays or pauses, one tap shows or hides the controls.
 */
@Composable
internal fun PictureGestures(ui: PlayerUi, moves: PlayerMoves) {
    Row(
        // A drag across the picture seeks: the bar and its preview follow the
        // finger, and letting go lands there. The whole width is two minutes
        // (a tenth of a long film, all of a short clip), so it is a fine control.
        // Keyed on the moves: the next episode's player brings new ones, and a gesture
        // detector started for the old player would keep seeking a released one.
        Modifier.fillMaxSize().pointerInput(moves) {
            detectHorizontalDragGestures(
                onDragStart = { if (moves.totalSec() > 0) { ui.settings = false; ui.tracksPanel = 0; ui.scrub = (moves.nowSec() / moves.totalSec()).toFloat().coerceIn(0f, 1f) } },
                onDragEnd = {
                    ui.scrub?.let { moves.seekToSec(it * moves.totalSec()) }
                    ui.scrub = null
                },
                onDragCancel = { ui.scrub = null },
            ) { change, dx ->
                val total = moves.totalSec()
                val from = ui.scrub
                if (total > 0 && from != null) {
                    val span = minOf(total, maxOf(120.0, total * 0.1))
                    ui.scrub = (from + dx / size.width * (span / total).toFloat()).coerceIn(0f, 1f)
                    change.consume()
                }
            }
        },
    ) {
        for (side in listOf(-1, 0, 1)) {
            Box(
                Modifier.weight(if (side == 0) 0.28f else 0.36f).fillMaxHeight().pointerInput(side, moves) {
                    detectTapGestures(
                        onDoubleTap = { if (side == 0) moves.togglePlay() else moves.skip(side) },
                        onTap = {
                            when {
                                side != 0 && System.currentTimeMillis() - ui.lastSkip < 700 -> moves.skip(side)
                                ui.settings -> ui.settings = false
                                else -> { ui.controls = !ui.controls; ui.touched++ }
                            }
                        },
                    )
                },
                contentAlignment = Alignment.Center,
            ) {
                val f = ui.flash
                if (f != null && f.first == side) SkipFlash(side, f.second)
            }
        }
    }
}

/** A skip you can see: the side lights up and says how far. */
@Composable
private fun SkipFlash(side: Int, seconds: Int) {
    Box(Modifier.fillMaxSize().background(Color.White.copy(alpha = 0.12f)), contentAlignment = Alignment.Center) {
        Row(
            Modifier.clip(CircleShape).background(Color.Black.copy(alpha = 0.7f)).padding(horizontal = 18.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(if (side < 0) Ic.Back10 else Ic.Fwd10, null, tint = Color.White, modifier = Modifier.size(22.dp))
            Text((if (side < 0) "−" else "+") + seconds + " s", color = Color.White, fontSize = 17.sp, fontWeight = FontWeight.Bold)
        }
    }
}

/**
 * The transport, in the middle of the picture where a thumb lands: back,
 * play or pause, forward. It is the one play target; the bar below keeps
 * time and options. Over a paused picture it stays, and the picture dims.
 */
@Composable
internal fun BoxScope.CentreTransport(ui: PlayerUi, moves: PlayerMoves, shown: Boolean, idle: Boolean, loading: Boolean) {
    if ((shown || idle) && ui.countdown == null && !ui.inPip) Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = if (idle) 0.38f else 0.26f)))
    if (shown && !ui.buffering && !loading && ui.countdown == null && ui.scrub == null && !ui.settings && ui.tracksPanel == 0) {
        Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(36.dp)) {
                SkipButton(Ic.Back10, moves.skipSec(), stringResource(R.string.player_skip_back, moves.skipSec())) { moves.seekBy(-moves.skipMs()); ui.touched++ }
                Box(
                    // Size, not a ring, sets it apart from the skips: the three discs are one family.
                    Modifier.size(76.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.72f))
                        .clickable(onClickLabel = stringResource(if (ui.playing) R.string.action_pause else R.string.action_play)) { moves.togglePlay() },
                    contentAlignment = Alignment.Center,
                ) { Icon(if (ui.ended) Ic.RotateCcw else if (ui.playing) Ic.Pause else Ic.Play, stringResource(if (ui.playing) R.string.action_pause else R.string.action_play), tint = Color.White, modifier = Modifier.size(32.dp)) }
                SkipButton(Ic.Fwd10, moves.skipSec(), stringResource(R.string.player_skip_fwd, moves.skipSec())) { moves.seekBy(moves.skipMs()); ui.touched++ }
            }
            // The end says what the button does.
            if (ui.ended) {
                Text(
                    stringResource(R.string.play_again),
                    Modifier.padding(top = 10.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.55f)).padding(horizontal = 12.dp, vertical = 5.dp),
                    color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.Medium,
                )
            }
        }
    }
}

/** The skip buttons: the turning arrow with the step inside. */
@Composable
private fun SkipButton(icon: ImageVector, seconds: Int, label: String, onClick: () -> Unit) {
    // Each says which way it goes: the two share a number, not a name.
    Box(
        Modifier.size(56.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.62f)).clickable(onClickLabel = label, onClick = onClick).semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, null, tint = Color.White, modifier = Modifier.size(30.dp))
        Text(seconds.toString(), Modifier.padding(top = 2.dp), color = Color.White, fontSize = 9.sp, fontWeight = FontWeight.Bold)
    }
}

/** Header: the way back and the title, over the picture. */
@Composable
internal fun BoxScope.PlayerTopBar(title: String, state: PlayerState, onBack: () -> Unit) {
    val l = LocalLain.current
    Column(
        Modifier.align(Alignment.TopStart).fillMaxWidth()
            .background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.65f), Color.Transparent))).safeDrawingPadding().padding(horizontal = 8.dp, vertical = 6.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconBtn(Ic.ArrowLeft, stringResource(R.string.player_back), onBack, tint = Color.White)
            Text(title, Modifier.weight(1f).padding(start = 4.dp), color = Color.White, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        if (state.loading || state.savedLocally) {
            Row(Modifier.padding(start = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                if (state.loading) Mono(stringResource(R.string.player_preparing), color = Color.White)
                if (state.savedLocally) Mono(stringResource(R.string.player_saved_locally), color = l.warn)
            }
        }
        ErrorLine(state.error, Modifier.padding(start = 16.dp, top = 6.dp))
    }
}

/** The gestures, said once. */
@Composable
internal fun BoxScope.GestureHint() {
    Text(
        stringResource(R.string.player_hint),
        Modifier.align(Alignment.Center).padding(horizontal = 32.dp).clip(RoundedCornerShape(10.dp)).background(Color.Black.copy(alpha = 0.7f)).padding(horizontal = 16.dp, vertical = 10.dp),
        color = Color.White, fontSize = 13.sp, lineHeight = 18.sp, textAlign = TextAlign.Center,
    )
}

/**
 * A short line over the picture, under the title, that fades after 2.5 s: what just changed.
 * It replaced a system toast, which sat on the bar's own buttons (over the Subtitles button
 * just used) and carried the app's icon into the film.
 */
@Composable
internal fun NoteLine(note: MutableState<String?>, shown: Boolean, modifier: Modifier) {
    val text = note.value
    LaunchedEffect(text) {
        if (text != null) {
            delay(2500)
            note.value = null
        }
    }
    if (text == null || !shown) return
    Text(
        text,
        modifier.safeDrawingPadding().padding(top = 64.dp, start = 24.dp, end = 24.dp)
            .clip(CircleShape).background(Color.Black.copy(alpha = 0.75f)).padding(horizontal = 16.dp, vertical = 8.dp),
        color = Color.White, fontSize = 14.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis,
    )
}

/** The end of an episode: the next one starts by itself unless stopped. */
@Composable
internal fun BoxScope.NextCountdown(seconds: Int, onCancel: () -> Unit, onPlay: () -> Unit) {
    Column(
        Modifier.align(Alignment.Center).clip(RoundedCornerShape(12.dp)).background(Color.Black.copy(alpha = 0.78f)).padding(horizontal = 22.dp, vertical = 18.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(stringResource(R.string.player_next_in, seconds), color = Color.White, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
        Row(Modifier.padding(top = 14.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            WButton(stringResource(R.string.player_cancel), onCancel, kind = Btn.Secondary)
            WButton(stringResource(R.string.action_play), onPlay, icon = Ic.Play)
        }
    }
}
