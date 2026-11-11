package io.github.enrell.lain.ui.screens

import android.app.Activity
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
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.media3.common.PlaybackException
import androidx.media3.exoplayer.ExoPlaybackException
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
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
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
    val view = LocalView.current

    // The player owns the whole screen: the system bars hide (a swipe
    // shows them for a moment) and the display stays on while it is up.
    DisposableEffect(view) {
        val bars = (context as? Activity)?.window?.let { WindowCompat.getInsetsController(it, view) }
        bars?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars?.hide(WindowInsetsCompat.Type.systemBars())
        view.keepScreenOn = true
        onDispose {
            bars?.show(WindowInsetsCompat.Type.systemBars())
            view.keepScreenOn = false
        }
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

            override fun onPlayerError(error: PlaybackException) {
                val mime = (error as? ExoPlaybackException)?.rendererFormat?.sampleMimeType
                val retried = error.errorCode in DECODER_ERRORS && model.decoderFailed(mime, player.currentPosition, player.duration)
                if (!retried) model.playerFailed(error)
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
        AndroidView(
            factory = { ctx ->
                PlayerView(ctx).apply {
                    layoutParams = ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT)
                    this.player = player
                    useController = true
                    setShowSubtitleButton(true)
                    controllerAutoShow = true
                    controllerShowTimeoutMs = l.tokens.autohideMs.toInt()
                }
            },
            modifier = Modifier.fillMaxSize(),
        )
        Column(
            Modifier.align(Alignment.TopStart).fillMaxWidth().background(l.controlsBg).safeDrawingPadding().padding(l.sp(3)),
            verticalArrangement = Arrangement.spacedBy(l.sp(1)),
        ) {
            Text(title, style = MaterialTheme.typography.titleMedium, color = Color.White)
            Row(horizontalArrangement = Arrangement.spacedBy(l.sp(2)), verticalAlignment = Alignment.CenterVertically) {
                when {
                    state.loading -> Label(stringResource(R.string.player_preparing), color = Color.White)
                    src != null -> Label(src.note, color = Color.White)
                }
                if (state.savedLocally) Label(stringResource(R.string.player_saved_locally), color = l.warn)
            }
            // The keys sit on their own row: a long note must not squeeze them.
            Row(horizontalArrangement = Arrangement.spacedBy(l.sp(2)), verticalAlignment = Alignment.CenterVertically) {
                Kbd("Space")
                Kbd("← →")
                Kbd("Esc")
            }
            val burnable = model.burnable()
            if (burnable.isNotEmpty()) {
                Row(horizontalArrangement = Arrangement.spacedBy(l.sp(2)), verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.player_ass_note), color = Color.White, style = MaterialTheme.typography.bodySmall, modifier = Modifier.weight(1f))
                    Action(stringResource(R.string.player_burn_in, burnable.first().label), "B", { model.burnIn(burnable.first(), player.currentPosition) })
                }
            }
            ErrorLine(state.error)
        }
    }
}

/** Errors where the device's decoder, not the file or network, gave up. */
private val DECODER_ERRORS = setOf(
    PlaybackException.ERROR_CODE_DECODER_INIT_FAILED,
    PlaybackException.ERROR_CODE_DECODER_QUERY_FAILED,
    PlaybackException.ERROR_CODE_DECODING_FAILED,
    PlaybackException.ERROR_CODE_DECODING_FORMAT_EXCEEDS_CAPABILITIES,
    PlaybackException.ERROR_CODE_DECODING_FORMAT_UNSUPPORTED,
)
