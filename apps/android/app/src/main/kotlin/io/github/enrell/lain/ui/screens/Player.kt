package io.github.enrell.lain.ui.screens

import android.view.View
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import android.net.Uri
import android.view.ViewGroup
import androidx.annotation.OptIn
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import io.github.enrell.lain.R
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.core.PlaySource
import io.github.enrell.lain.core.SourceKind
import io.github.enrell.lain.player.DeviceCaps
import io.github.enrell.lain.ui.PlayerModel
import io.github.enrell.lain.ui.theme.LocalLain
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import java.io.File

/** How often a playing position is saved (D-182). */
private const val SAVE_EVERY_MS = 5_000L

private fun mediaItem(src: PlaySource): MediaItem {
    val uri = if (src.kind == SourceKind.OFFLINE) Uri.fromFile(File(src.url)) else Uri.parse(src.url)
    val subs = src.subtitles.map { t ->
        MediaItem.SubtitleConfiguration.Builder(Uri.parse(t.url))
            .setMimeType(MimeTypes.TEXT_VTT)
            .setLanguage(t.language.takeIf { it != "und" })
            .setLabel(t.label)
            .setSelectionFlags(if (t.forced) C.SELECTION_FLAG_FORCED else 0)
            .build()
    }
    val b = MediaItem.Builder().setUri(uri).setSubtitleConfigurations(subs)
    if (src.kind == SourceKind.HLS) b.setMimeType(MimeTypes.APPLICATION_M3U8)
    return b.build()
}

/**
 * Media3/ExoPlayer with hardware decoding (D-246). Keys: Space plays or
 * pauses, ←/→ seek 10 s, B burns typeset subtitles in, Esc closes.
 */
@OptIn(UnstableApi::class)
@Composable
fun PlayerScreen(core: LainCoreInterface, itemId: String, title: String, preferredLanguage: String) {
    val l = LocalLain.current
    val context = LocalContext.current
    val scope = remember(itemId) { CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate) }
    val model = remember(itemId) { PlayerModel(core, itemId, DeviceCaps.claim, scope) }
    val state by model.state.collectAsState()
    val player = remember(itemId) {
        ExoPlayer.Builder(context).build().apply {
            if (preferredLanguage.isNotEmpty()) {
                trackSelectionParameters = trackSelectionParameters.buildUpon().setPreferredTextLanguage(preferredLanguage).build()
            }
        }
    }
    val focus = remember { FocusRequester() }
    val back = androidx.activity.compose.LocalOnBackPressedDispatcherOwner.current?.onBackPressedDispatcher

    // The picture takes the whole screen; a swipe from an edge brings the
    // system bars back for a moment.
    DisposableEffect(Unit) {
        val window = (context as? android.app.Activity)?.window
        val bars = window?.let { androidx.core.view.WindowCompat.getInsetsController(it, it.decorView) }
        bars?.systemBarsBehavior = androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars?.hide(androidx.core.view.WindowInsetsCompat.Type.systemBars())
        onDispose { bars?.show(androidx.core.view.WindowInsetsCompat.Type.systemBars()) }
    }
    LaunchedEffect(itemId) { model.load() }
    DisposableEffect(itemId) {
        val listener = object : Player.Listener {
            override fun onPlaybackStateChanged(playbackState: Int) {
                if (playbackState == Player.STATE_ENDED) model.report(player.currentPosition, player.duration, ended = true)
            }

            override fun onIsPlayingChanged(isPlaying: Boolean) {
                if (!isPlaying) model.report(player.currentPosition, player.duration)
            }
        }
        player.addListener(listener)
        onDispose {
            player.removeListener(listener)
            model.close(player.currentPosition, player.duration).invokeOnCompletion { scope.cancel() }
            player.release()
        }
    }
    val src = state.source
    LaunchedEffect(src) {
        if (src == null) return@LaunchedEffect
        player.setMediaItem(mediaItem(src), model.seekOnStartMs(src))
        player.prepare()
        player.playWhenReady = true
        focus.requestFocus()
        while (true) {
            delay(SAVE_EVERY_MS)
            if (player.isPlaying) model.report(player.currentPosition, player.duration)
        }
    }

    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black)
            .focusRequester(focus)
            .focusable()
            .onPreviewKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when (e.key) {
                    Key.Spacebar, Key.MediaPlayPause -> {
                        if (player.isPlaying) player.pause() else player.play()
                        true
                    }
                    Key.DirectionLeft -> {
                        player.seekTo((player.currentPosition - 10_000).coerceAtLeast(0))
                        true
                    }
                    Key.DirectionRight -> {
                        player.seekTo(player.currentPosition + 10_000)
                        true
                    }
                    Key.B -> {
                        model.burnable().firstOrNull()?.let { model.burnIn(it, player.currentPosition) }
                        true
                    }
                    else -> false
                }
            },
    ) {
        var controls by remember { mutableStateOf(true) }
        var view by remember { mutableStateOf<PlayerView?>(null) }
        // The running total of a burst of skips, and its side (-1 back, 1 forward).
        var flash by remember { mutableStateOf<Pair<Int, Int>?>(null) }
        var lastSkip by remember { mutableStateOf(0L) }
        LaunchedEffect(flash) {
            if (flash != null) {
                delay(700)
                flash = null
            }
        }
        fun skip(side: Int) {
            val to = player.currentPosition + side * SKIP_MS
            player.seekTo(if (player.duration > 0) to.coerceIn(0, player.duration) else to.coerceAtLeast(0))
            flash = side to ((flash?.takeIf { it.first == side }?.second ?: 0) + (SKIP_MS / 1000).toInt())
            lastSkip = System.currentTimeMillis()
        }
        fun toggleControls() {
            view?.let { if (it.isControllerFullyVisible) it.hideController() else it.showController() }
        }
        AndroidView(
            factory = { ctx ->
                PlayerView(ctx).apply {
                    layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
                    this.player = player
                    useController = true
                    setShowSubtitleButton(true)
                    // Skipping is a double tap on the picture; the bar keeps play and seek.
                    setShowRewindButton(false)
                    setShowFastForwardButton(false)
                    setShowPreviousButton(false)
                    setShowNextButton(false)
                    controllerAutoShow = true
                    controllerShowTimeoutMs = l.tokens.autohideMs.toInt()
                    setControllerVisibilityListener(PlayerView.ControllerVisibilityListener { v -> controls = v == View.VISIBLE })
                    view = this
                }
            },
            modifier = Modifier.fillMaxSize(),
        )
        /*
         * The picture's gestures, as in the web player: a double tap on the
         * left or right third seeks 10 s and keeps counting while the taps
         * continue, without bringing the controls up; one tap shows or hides
         * them. The middle third is the bar's own while it is showing.
         */
        Row(Modifier.fillMaxSize().padding(top = 72.dp, bottom = 112.dp)) {
            for (side in listOf(-1, 0, 1)) {
                Box(
                    Modifier.weight(if (side == 0) 0.28f else 0.36f).fillMaxHeight().then(
                        if (side == 0 && controls) Modifier else Modifier.pointerInput(side) {
                            detectTapGestures(
                                onDoubleTap = { if (side == 0) { if (player.isPlaying) player.pause() else player.play() } else skip(side) },
                                onTap = { if (side != 0 && System.currentTimeMillis() - lastSkip < 700) skip(side) else toggleControls() },
                            )
                        },
                    ),
                    contentAlignment = Alignment.Center,
                ) {
                    val f = flash
                    if (f != null && f.first == side) {
                        Box(Modifier.fillMaxSize().background(Color.White.copy(alpha = 0.08f)), contentAlignment = Alignment.Center) {
                            Text(
                                (if (side < 0) "−" else "+") + f.second + "s",
                                Modifier.clip(CircleShape).background(Color.Black.copy(alpha = 0.55f)).padding(horizontal = 14.dp, vertical = 6.dp),
                                color = Color.White, fontSize = 14.sp, fontWeight = FontWeight.SemiBold,
                            )
                        }
                    }
                }
            }
        }
        // The header, with the controls: the way back and the title.
        if (controls || state.loading || state.error != null) {
            Column(
                Modifier.align(Alignment.TopStart).fillMaxWidth()
                    .background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.6f), Color.Transparent))).statusBarsPadding().padding(horizontal = 6.dp, vertical = 6.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    IconBtn(Ic.ArrowLeft, "Back", { back?.onBackPressed() }, tint = Color.White)
                    Text(title, Modifier.weight(1f), color = Color.White, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Row(Modifier.padding(start = 14.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                    when {
                        state.loading -> Mono(stringResource(R.string.player_preparing), color = Color.White)
                        src != null && src.note.isNotBlank() -> Mono(src.note, color = Color.White.copy(alpha = 0.7f))
                    }
                    if (state.savedLocally) Mono(stringResource(R.string.player_saved_locally), color = l.warn)
                }
                val burnable = model.burnable()
                if (burnable.isNotEmpty()) {
                    Row(Modifier.padding(start = 14.dp, end = 8.dp, top = 6.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(stringResource(R.string.player_ass_note), color = Color.White, fontSize = 12.sp, modifier = Modifier.weight(1f))
                        WButton(stringResource(R.string.player_burn_in, burnable.first().label), { model.burnIn(burnable.first(), player.currentPosition) }, kind = Btn.Secondary)
                    }
                }
                ErrorLine(state.error, Modifier.padding(start = 14.dp, top = 6.dp))
            }
        }
    }
}

private const val SKIP_MS = 10_000L
