package io.github.enrell.lain.ui.screens

import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.contentDescription
import io.github.enrell.lain.core.Item
import androidx.media3.common.PlaybackException
import androidx.media3.exoplayer.ExoPlaybackException
import io.github.enrell.lain.domain.SKIP_CHOICES
import io.github.enrell.lain.domain.skipSeconds
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import io.github.enrell.lain.Board
import io.github.enrell.lain.player.Anime4kEffect
import io.github.enrell.lain.player.VideoEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.layout.layout
import androidx.compose.foundation.Image
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.unit.coerceAtLeast
import androidx.compose.ui.unit.coerceIn
import androidx.media3.common.TrackSelectionOverride
import androidx.media3.common.Tracks
import io.github.enrell.lain.graph
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
 * The player, with the web player's chrome and gestures (D-246 for the
 * decoding): a header with the way back and the title; a seek bar that
 * shows the frame it points at while dragged; play, 10 s back and forward,
 * mute, settings (speed, subtitles) and fullscreen underneath. A double
 * tap on the left or right third seeks 10 s and keeps counting while the
 * taps continue, without bringing the controls up; one tap shows or hides
 * them. Keys: Space, ←/→, B burns typeset subtitles in, Esc closes.
 */
@OptIn(UnstableApi::class)
@Composable
fun PlayerScreen(
    core: LainCoreInterface,
    itemId: String,
    title: String,
    preferredLanguage: String,
    /** What plays after this one (the title's next episode), if anything. */
    next: Item? = null,
    onNext: (Item) -> Unit = {},
) {
    val l = LocalLain.current
    val context = LocalContext.current
    val graph = context.graph
    val activity = context as? android.app.Activity
    val scope = remember(itemId) { CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate) }
    val model = remember(itemId) {
        PlayerModel(core, itemId, DeviceCaps.claim, scope, graph.prefs.failedFamilies(itemId)) { family -> graph.prefs.rememberFailed(itemId, family) }
    }
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

    // What the chrome shows, read from the player.
    var playing by remember { mutableStateOf(false) }
    var buffering by remember { mutableStateOf(true) }
    var ended by remember { mutableStateOf(false) }
    var positionMs by remember { mutableStateOf(0L) }
    var durationMs by remember { mutableStateOf(0L) }
    var bufferedMs by remember { mutableStateOf(0L) }
    var muted by remember { mutableStateOf(false) }
    var speed by remember { mutableStateOf(1f) }
    var textTracks by remember { mutableStateOf(listOf<TextTrack>()) }
    var textOff by remember { mutableStateOf(false) }
    var audioTracks by remember { mutableStateOf(listOf<TextTrack>()) }
    // Which list is open over the picture: 0 none, 1 subtitles, 2 audio.
    var tracksPanel by remember { mutableStateOf(0) }
    var showRemaining by remember { mutableStateOf(false) }
    // Seconds until the next episode starts by itself, once this one ends.
    var countdown by remember { mutableStateOf<Int?>(null) }
    var hint by remember { mutableStateOf(!graph.prefs.playerHintSeen) }

    var controls by remember { mutableStateOf(true) }
    var settings by remember { mutableStateOf(false) }
    var landscape by remember { mutableStateOf(true) }
    // Dragging the seek bar: the fraction under the finger, or null.
    var scrub by remember { mutableStateOf<Float?>(null) }
    var touched by remember { mutableStateOf(0L) }
    var preview by remember { mutableStateOf<ImageBitmap?>(null) }
    // The storyboard, once it has arrived: previews are cut out of it.
    var board by remember { mutableStateOf<Board?>(null) }
    // Where a seek that needs a new session is headed, until it plays.
    var pendingSec by remember { mutableStateOf<Double?>(null) }
    var effect by remember { mutableStateOf(VideoEffect.of(graph.prefs.videoEffect)) }
    var effectError by remember { mutableStateOf<String?>(null) }
    var effectsOn by remember { mutableStateOf(false) }
    var skipChoice by remember { mutableStateOf(graph.prefs.skipSec) }
    // The size the picture is shown at, read by the effect on the GL thread.
    val shownAt = remember { java.util.concurrent.atomic.AtomicReference(0 to 0) }
    // The running total of a burst of skips, and its side (-1 back, 1 forward).
    var flash by remember { mutableStateOf<Pair<Int, Int>?>(null) }
    var lastSkip by remember { mutableStateOf(0L) }

    // The picture takes the whole screen; a swipe from an edge brings the
    // system bars back for a moment.
    DisposableEffect(Unit) {
        val window = activity?.window
        val bars = window?.let { androidx.core.view.WindowCompat.getInsetsController(it, it.decorView) }
        bars?.systemBarsBehavior = androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        bars?.hide(androidx.core.view.WindowInsetsCompat.Type.systemBars())
        window?.addFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        activity?.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        onDispose {
            bars?.show(androidx.core.view.WindowInsetsCompat.Type.systemBars())
            window?.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            activity?.requestedOrientation = restOrientation(activity)
        }
    }
    LaunchedEffect(itemId) { model.load() }
    DisposableEffect(itemId) {
        val listener = object : Player.Listener {
            override fun onPlaybackStateChanged(playbackState: Int) {
                buffering = playbackState == Player.STATE_BUFFERING
                ended = playbackState == Player.STATE_ENDED
                if (ended) model.report(player.currentPosition, player.duration, ended = true)
            }

            override fun onIsPlayingChanged(isPlaying: Boolean) {
                playing = isPlaying
                if (!isPlaying) model.report(player.currentPosition, player.duration)
            }

            override fun onPlayerError(error: PlaybackException) {
                // The device's decoder gave up on a codec it claimed: the server
                // transcodes from here instead. Anything else is shown.
                val mime = (error as? ExoPlaybackException)?.rendererFormat?.sampleMimeType
                val retried = error.errorCode in DECODER_ERRORS && model.decoderFailed(mime, player.currentPosition, player.duration)
                if (!retried) model.playerFailed(error)
            }

            override fun onTracksChanged(tracks: Tracks) {
                fun of(type: Int) = tracks.groups.filter { it.type == type }.flatMap { g ->
                    (0 until g.length).map { i ->
                        val f = g.getTrackFormat(i)
                        val name = listOfNotNull(f.label, f.language?.takeIf { it != "und" }?.uppercase()).distinct().joinToString(" · ")
                        TextTrack(g, i, name.ifEmpty { "#${i + 1}" }, g.isTrackSelected(i))
                    }
                }
                textTracks = of(C.TRACK_TYPE_TEXT)
                audioTracks = of(C.TRACK_TYPE_AUDIO)
            }

            override fun onVideoSizeChanged(size: androidx.media3.common.VideoSize) {
                // A picture that is taller than wide is watched upright.
                if (size.width > 0 && size.height > size.width && landscape) {
                    landscape = false
                    activity?.requestedOrientation = restOrientation(activity)
                }
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
        pendingSec = null
        positionMs = model.seekOnStartMs(src)
        focus.requestFocus()
        while (true) {
            delay(SAVE_EVERY_MS)
            if (player.isPlaying) model.report(player.currentPosition, player.duration)
        }
    }
    // The storyboard is asked for once the play is known; until it is
    // here the preview falls back to single frames.
    LaunchedEffect(itemId, src != null) {
        if (src != null && src.kind != SourceKind.OFFLINE && board == null) board = graph.storyboard(itemId)
    }
    // The video effect: Media3 runs the shaders on each decoded frame.
    LaunchedEffect(effect, src != null) {
        if (src == null) return@LaunchedEffect
        if (effect == VideoEffect.Off) {
            if (effectsOn) player.setVideoEffects(emptyList())
            effectsOn = false
        } else {
            val main = android.os.Handler(android.os.Looper.getMainLooper())
            player.setVideoEffects(
                listOf(
                    Anime4kEffect(effect, { shownAt.get() }) { reason ->
                        main.post {
                            effectError = reason
                            effect = VideoEffect.Off
                        }
                    },
                ),
            )
            effectsOn = true
        }
    }
    // The clock the chrome reads: often enough for a smooth bar.
    LaunchedEffect(player) {
        while (true) {
            positionMs = player.currentPosition
            durationMs = player.duration.coerceAtLeast(0)
            bufferedMs = player.bufferedPosition
            delay(250)
        }
    }
    // The controls leave after a while of playing untouched.
    // Paused or ended is a state you can see: the controls stay until it plays again.
    val idle = !playing && !buffering && !state.loading && state.source != null
    val pinned = idle || settings || tracksPanel != 0 || scrub != null || state.error != null || state.loading || countdown != null
    LaunchedEffect(controls, playing, pinned, touched) {
        if (controls && playing && !pinned) {
            // Long enough to find a button and press it: a second and a half was a race.
            delay(l.tokens.autohideMs.toLong().coerceAtLeast(4500))
            controls = false
            settings = false
        }
    }

    // Everything the bar shows and takes is in source seconds: the whole
    // title, whatever part of it the current session holds.
    fun totalSec(): Double = model.durationSec(durationMs)
    fun nowSec(): Double = pendingSec ?: model.sourceSec(positionMs)
    fun seekToSec(target: Double) {
        val total = totalSec()
        val t = if (total > 0) target.coerceIn(0.0, total) else target.coerceAtLeast(0.0)
        if (model.needsRebuild(t, player.duration)) {
            // Outside what this session holds: a new one that begins there.
            pendingSec = t
            model.rebuild(t)
            return
        }
        val offset = state.source?.offsetSec ?: 0.0
        player.seekTo(((t - offset) * 1000).toLong().coerceAtLeast(0))
        positionMs = player.currentPosition
    }
    fun seekBy(deltaMs: Long) = seekToSec(nowSec() + deltaMs / 1000.0)
    fun skipSec(): Int = skipSeconds(skipChoice, totalSec())
    fun skipMs(): Long = skipSec() * 1000L
    fun skip(side: Int) {
        seekBy(side * skipMs())
        flash = side to ((flash?.takeIf { it.first == side }?.second ?: 0) + skipSec())
        lastSkip = System.currentTimeMillis()
    }
    fun togglePlay() {
        when {
            ended -> { player.seekTo(0); player.play() }
            player.isPlaying -> player.pause()
            else -> player.play()
        }
        touched++
    }
    LaunchedEffect(flash) {
        if (flash != null) {
            delay(900)
            flash = null
        }
    }
    LaunchedEffect(ended, next?.id) {
        if (ended && next != null) {
            for (n in 6 downTo 1) {
                countdown = n
                delay(1000)
                if (countdown == null) return@LaunchedEffect
            }
            countdown = null
            onNext(next)
        } else {
            countdown = null
        }
    }
    LaunchedEffect(hint, playing) {
        if (hint && playing) {
            delay(4500)
            hint = false
            graph.prefs.playerHintSeen = true
        }
    }

    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black)
            .onSizeChanged { shownAt.set(it.width to it.height) }
            .focusRequester(focus)
            .focusable()
            .onPreviewKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when (e.key) {
                    Key.Spacebar, Key.MediaPlayPause -> { togglePlay(); true }
                    Key.DirectionLeft -> { seekBy(-skipMs()); true }
                    Key.DirectionRight -> { seekBy(skipMs()); true }
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
                    // The chrome is the app's own, drawn like the web player's.
                    useController = false
                }
            },
            modifier = Modifier.fillMaxSize(),
        )

        // The picture's gestures.
        Row(Modifier.fillMaxSize()) {
            for (side in listOf(-1, 0, 1)) {
                Box(
                    Modifier.weight(if (side == 0) 0.28f else 0.36f).fillMaxHeight().pointerInput(side) {
                        detectTapGestures(
                            onDoubleTap = { if (side == 0) togglePlay() else skip(side) },
                            onTap = {
                                when {
                                    side != 0 && System.currentTimeMillis() - lastSkip < 700 -> skip(side)
                                    settings -> settings = false
                                    else -> { controls = !controls; touched++ }
                                }
                            },
                        )
                    },
                    contentAlignment = Alignment.Center,
                ) {
                    val f = flash
                    if (f != null && f.first == side) {
                        // A skip you can see: the side lights up and says how far.
                        Box(Modifier.fillMaxSize().background(Color.White.copy(alpha = 0.12f)), contentAlignment = Alignment.Center) {
                            Row(
                                Modifier.clip(CircleShape).background(Color.Black.copy(alpha = 0.7f)).padding(horizontal = 18.dp, vertical = 10.dp),
                                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                Icon(if (side < 0) Ic.Back10 else Ic.Fwd10, null, tint = Color.White, modifier = Modifier.size(22.dp))
                                Text((if (side < 0) "−" else "+") + f.second + " s", color = Color.White, fontSize = 17.sp, fontWeight = FontWeight.Bold)
                            }
                        }
                    }
                }
            }
        }

        if (buffering || state.loading) {
            CircularProgressIndicator(Modifier.align(Alignment.Center).size(44.dp), color = Color.White, strokeWidth = 3.dp)
        }

        val shown = controls || pinned
        // The transport, in the middle of the picture where a thumb lands: back,
        // play or pause, forward. It is the one play target; the bar below keeps
        // time and options. Over a paused picture it stays, and the picture dims.
        if (idle && countdown == null) Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.28f)))
        if (shown && !buffering && !state.loading && countdown == null && scrub == null && !settings && tracksPanel == 0) {
            Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(36.dp)) {
                    SkipButton(Ic.Back10, skipSec(), stringResource(R.string.player_skip_back, skipSec())) { seekBy(-skipMs()); touched++ }
                    Box(
                        Modifier.size(76.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.55f)).border(1.dp, Color.White.copy(alpha = 0.3f), CircleShape)
                            .clickable(onClickLabel = stringResource(if (playing) R.string.action_pause else R.string.action_play)) { togglePlay() },
                        contentAlignment = Alignment.Center,
                    ) { Icon(if (ended) Ic.RotateCcw else if (playing) Ic.Pause else Ic.Play, stringResource(if (playing) R.string.action_pause else R.string.action_play), tint = Color.White, modifier = Modifier.size(32.dp)) }
                    SkipButton(Ic.Fwd10, skipSec(), stringResource(R.string.player_skip_fwd, skipSec())) { seekBy(skipMs()); touched++ }
                }
                // The end says what the button does.
                if (ended) {
                    Text(
                        stringResource(R.string.play_again),
                        Modifier.padding(top = 10.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.55f)).padding(horizontal = 12.dp, vertical = 5.dp),
                        color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.Medium,
                    )
                }
            }
        }

        // Header: the way back and the title, over the picture.
        if (shown) {
            Column(
                Modifier.align(Alignment.TopStart).fillMaxWidth()
                    .background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.65f), Color.Transparent))).safeDrawingPadding().padding(horizontal = 8.dp, vertical = 6.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    IconBtn(Ic.ArrowLeft, stringResource(R.string.player_back), { back?.onBackPressed() }, tint = Color.White)
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
        // The gestures, said once.
        if (hint && playing && !pinned) {
            Text(
                stringResource(R.string.player_hint),
                Modifier.align(Alignment.Center).padding(horizontal = 32.dp).clip(RoundedCornerShape(10.dp)).background(Color.Black.copy(alpha = 0.7f)).padding(horizontal = 16.dp, vertical = 10.dp),
                color = Color.White, fontSize = 13.sp, lineHeight = 18.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center,
            )
        }
        // The end of an episode: the next one starts by itself unless stopped.
        countdown?.let { n ->
            if (next != null) {
                Column(
                    Modifier.align(Alignment.Center).clip(RoundedCornerShape(12.dp)).background(Color.Black.copy(alpha = 0.78f)).padding(horizontal = 22.dp, vertical = 18.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    Text(stringResource(R.string.player_next_in, n), color = Color.White, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    Row(Modifier.padding(top = 14.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        WButton(stringResource(R.string.player_cancel), { countdown = null }, kind = Btn.Secondary)
                        WButton(stringResource(R.string.action_play), { countdown = null; onNext(next) }, icon = Ic.Play)
                    }
                }
            }
        }

        // Footer: the seek row, then the transport.
        if (shown) {
            val total = totalSec().coerceAtLeast(0.001)
            val at = scrub?.let { it * total } ?: nowSec()
            Column(
                Modifier.align(Alignment.BottomStart).fillMaxWidth()
                    .background(Brush.verticalGradient(0f to Color.Transparent, 0.45f to Color.Black.copy(alpha = 0.6f), 1f to Color.Black.copy(alpha = 0.88f))).safeDrawingPadding().padding(horizontal = 12.dp).padding(bottom = 10.dp, top = 36.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    BoxWithConstraints(Modifier.weight(1f)) {
                        val width = maxWidth
                        val fraction = (at / total).toFloat().coerceIn(0f, 1f)
                        // The frame the bar points at, while dragging it.
                        scrub?.let { f ->
                            val sec = f * total
                            val sheet = board
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
                                            graph.frame(itemId, want)?.let { preview = it }
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
                                        preview?.let { Image(it, null, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize()) }
                                    }
                                }
                                // Its time under it, on the same card: off the picture, and not floating.
                                Text(clock(sec), Modifier.padding(top = 5.dp, bottom = 1.dp), color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                            }
                        }
                        Box(
                            Modifier.fillMaxWidth().height(36.dp)
                                .pointerInput(Unit) {
                                    detectTapGestures { o ->
                                        if (totalSec() > 0) seekToSec(o.x / size.width * totalSec())
                                        touched++
                                    }
                                }
                                .pointerInput(Unit) {
                                    detectHorizontalDragGestures(
                                        onDragStart = { o -> settings = false; tracksPanel = 0; if (totalSec() > 0) scrub = (o.x / size.width).coerceIn(0f, 1f) },
                                        onDragEnd = {
                                            scrub?.let { seekToSec(it * totalSec()) }
                                            scrub = null
                                            touched++
                                        },
                                        onDragCancel = { scrub = null },
                                    ) { change, _ -> if (totalSec() > 0) scrub = (change.position.x / size.width).coerceIn(0f, 1f) }
                                },
                            contentAlignment = Alignment.CenterStart,
                        ) {
                            Box(Modifier.fillMaxWidth().height(5.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.25f)))
                            // What is loaded sits where it is: a session that began
                            // partway holds a range inside the title, not its start.
                            val heldFrom = ((state.source?.offsetSec ?: 0.0) / total).toFloat().coerceIn(0f, 1f)
                            val heldTo = (model.sourceSec(bufferedMs) / total).toFloat().coerceIn(heldFrom, 1f)
                            Box(Modifier.offset(x = width * heldFrom).width(width * (heldTo - heldFrom)).height(5.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.3f)))
                            Box(Modifier.fillMaxWidth(fraction).height(5.dp).clip(CircleShape).background(l.accent))
                            Box(Modifier.offset(x = (width - 18.dp) * fraction).size(if (scrub != null) 24.dp else 18.dp).clip(CircleShape).background(l.accent))
                        }
                    }
                    Text(
                        if (showRemaining) "−" + clock((totalSec() - at).coerceAtLeast(0.0)) else clock(at) + " / " + clock(totalSec()),
                        Modifier.padding(start = 6.dp).clip(l.buttonShape).clickable { showRemaining = !showRemaining; touched++ }.padding(horizontal = 6.dp, vertical = 8.dp),
                        color = Color.White, fontSize = 13.sp, fontWeight = FontWeight.Medium,
                    )
                }
                Row(Modifier.padding(top = 2.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    if (next != null) IconBtn(Ic.SkipNext, stringResource(R.string.player_next), { onNext(next) }, tint = Color.White)
                    if (speed != 1f) {
                        Text(
                            "$speed×",
                            Modifier.padding(start = 4.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.16f)).clickable { tracksPanel = 0; settings = true }.padding(horizontal = 10.dp, vertical = 5.dp),
                            color = Color.White, fontSize = 12.sp, fontWeight = FontWeight.SemiBold,
                        )
                    }
                    IconBtn(if (muted) Ic.Muted else Ic.Volume, "Mute", { muted = !muted; player.volume = if (muted) 0f else 1f; touched++ }, tint = Color.White)
                    Box(Modifier.weight(1f))
                    // Subtitles and audio are on the bar, not behind the gear.
                    if (textTracks.isNotEmpty()) {
                        IconBtn(Ic.Captions, stringResource(R.string.player_subtitles), { settings = false; tracksPanel = if (tracksPanel == 1) 0 else 1 }, tint = if (tracksPanel == 1 || (!textOff && textTracks.any { it.selected })) l.accent else Color.White)
                    }
                    if (audioTracks.size > 1) {
                        IconBtn(Ic.Audio, stringResource(R.string.player_audio), { settings = false; tracksPanel = if (tracksPanel == 2) 0 else 2 }, tint = if (tracksPanel == 2) l.accent else Color.White)
                    }
                    IconBtn(Ic.Gear, "Settings", { tracksPanel = 0; settings = !settings }, tint = if (settings) l.accent else Color.White)
                    IconBtn(Ic.Maximize, "Fullscreen", {
                        landscape = !landscape
                        activity?.requestedOrientation =
                            if (landscape) android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE else restOrientation(activity)
                        touched++
                    }, tint = Color.White)
                }
            }
        }

        // A list over the picture closes with a tap outside it.
        if (settings || tracksPanel != 0) {
            Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.55f)).pointerInput(Unit) { detectTapGestures { settings = false; tracksPanel = 0 } })
        }
        if (tracksPanel != 0) {
            val subs = tracksPanel == 1
            Column(
                Modifier.align(Alignment.BottomEnd).safeDrawingPadding().padding(end = 12.dp, bottom = 104.dp).width(264.dp).heightIn(max = 360.dp).clip(RoundedCornerShape(10.dp))
                    .verticalScroll(rememberScrollState()).background(l.bg).border(1.dp, l.line.copy(alpha = 0.6f), RoundedCornerShape(10.dp)).padding(8.dp),
            ) {
                Text(stringResource(if (subs) R.string.player_subtitles else R.string.player_audio), Modifier.padding(start = 8.dp, top = 6.dp, bottom = 8.dp), color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                if (subs) {
                    OptionRow(stringResource(R.string.player_off), null, textOff || textTracks.none { it.selected }, {
                        textOff = true
                        player.trackSelectionParameters = player.trackSelectionParameters.buildUpon().setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true).build()
                        tracksPanel = 0
                    })
                }
                for (t in if (subs) textTracks else audioTracks) {
                    OptionRow(t.label, null, t.selected && !(subs && textOff), {
                        val type = if (subs) C.TRACK_TYPE_TEXT else C.TRACK_TYPE_AUDIO
                        if (subs) textOff = false
                        player.trackSelectionParameters = player.trackSelectionParameters.buildUpon().setTrackTypeDisabled(type, false)
                            .setOverrideForType(TrackSelectionOverride(t.group.mediaTrackGroup, t.index)).build()
                        tracksPanel = 0
                    })
                }
            }
        }
        if (settings) {
            Column(
                Modifier.align(Alignment.BottomEnd).safeDrawingPadding().padding(end = 12.dp, bottom = 104.dp).width(280.dp).heightIn(max = 420.dp).clip(RoundedCornerShape(10.dp))
                    .verticalScroll(rememberScrollState()).background(l.bg).border(1.dp, l.line.copy(alpha = 0.6f), RoundedCornerShape(10.dp)).padding(horizontal = 14.dp, vertical = 8.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.player_playback), Modifier.weight(1f), color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
                    IconBtn(Ic.Close, "Close", { settings = false }, tint = l.text)
                }
                PickRow(
                    stringResource(R.string.player_speed),
                    if (speed == 1f) stringResource(R.string.player_normal) else "$speed×",
                    SPEEDS.map { (if (it == 1f) stringResource(R.string.player_normal) else "$it×") to (it == speed) },
                ) { i ->
                    speed = SPEEDS[i]
                    player.setPlaybackSpeed(speed)
                }
                if (src?.kind == SourceKind.HLS && state.qualities.isNotEmpty()) {
                    val auto = stringResource(R.string.player_auto)
                    PickRow(
                        stringResource(R.string.player_quality),
                        state.quality.ifEmpty { auto },
                        listOf(auto to state.quality.isEmpty()) + state.qualities.map { q ->
                            (q.name + when {
                                q.bitrateKbps >= 1000 -> " · ${q.bitrateKbps / 1000} Mbps"
                                q.bitrateKbps > 0 -> " · ${q.bitrateKbps} kbps"
                                else -> ""
                            }) to (q.name == state.quality)
                        },
                    ) { i ->
                        val pick = if (i == 0) "" else state.qualities[i - 1].name
                        if (pick != state.quality) {
                            pendingSec = nowSec()
                            model.rebuild(nowSec(), quality = pick)
                        }
                    }
                }
                val byLength = stringResource(R.string.player_skip_auto, skipSeconds(0, totalSec()))
                PickRow(
                    stringResource(R.string.player_skip),
                    if (skipChoice == 0) byLength else "$skipChoice s",
                    SKIP_CHOICES.map { (if (it == 0) byLength else "$it s") to (it == skipChoice) },
                ) { i ->
                    skipChoice = SKIP_CHOICES[i]
                    graph.prefs.skipSec = skipChoice
                }
                PickRow(
                    stringResource(R.string.player_effects),
                    effect.label,
                    VideoEffect.entries.map { it.label to (it == effect) },
                ) { i ->
                    effectError = null
                    effect = VideoEffect.entries[i]
                    graph.prefs.videoEffect = effect.id
                }
                effectError?.let { Text(stringResource(R.string.player_effect_failed, it), color = l.danger, fontSize = 12.sp, lineHeight = 16.sp) }
                Text(stringResource(R.string.player_effects_hint), Modifier.padding(bottom = 4.dp), color = l.muted, fontSize = 12.sp, lineHeight = 16.sp)
                // How it is reaching you, in words; the detail is the server's.
                src?.note?.takeIf { it.isNotBlank() }?.let { note ->
                    Text(
                        if (note.contains("direct", ignoreCase = true)) stringResource(R.string.player_original) else stringResource(R.string.player_converted, note.lowercase()),
                        Modifier.padding(top = 8.dp, bottom = 6.dp), color = l.muted, fontSize = 12.sp,
                    )
                }
                val burnable = model.burnable()
                if (burnable.isNotEmpty()) {
                    Text(stringResource(R.string.player_ass_note), Modifier.padding(top = 6.dp), color = l.muted, fontSize = 12.sp, lineHeight = 16.sp)
                    WButton(
                        stringResource(R.string.player_burn_in, burnable.first().label),
                        { settings = false; model.burnIn(burnable.first(), player.currentPosition) },
                        Modifier.fillMaxWidth().padding(vertical = 6.dp),
                        kind = Btn.Secondary,
                    )
                }
            }
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
private val SPEEDS = listOf(0.5f, 0.75f, 1f, 1.25f, 1.5f, 2f)

private class TextTrack(val group: Tracks.Group, val index: Int, val label: String, val selected: Boolean)

/** The skip buttons: the turning arrow with the step inside. */
@Composable
private fun SkipButton(icon: androidx.compose.ui.graphics.vector.ImageVector, seconds: Int, label: String, onClick: () -> Unit) {
    // Each says which way it goes: the two share a number, not a name.
    Box(
        Modifier.size(56.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.4f)).clickable(onClickLabel = label, onClick = onClick).semantics { contentDescription = label },
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, null, tint = Color.White, modifier = Modifier.size(30.dp))
        Text(seconds.toString(), Modifier.padding(top = 2.dp), color = Color.White, fontSize = 9.sp, fontWeight = FontWeight.Bold)
    }
}

/** One setting: its name, the current value, and the choices when opened. */
@Composable
private fun PickRow(label: String, value: String, options: List<Pair<String, Boolean>>, onPick: (Int) -> Unit) {
    val l = LocalLain.current
    var open by remember { mutableStateOf(false) }
    Column {
        Row(Modifier.fillMaxWidth().height(40.dp).clickable { open = !open }, verticalAlignment = Alignment.CenterVertically) {
            Text(label, Modifier.weight(1f), color = l.text, fontSize = 13.sp)
            Text(value, Modifier.clip(l.buttonShape).border(1.dp, l.line, l.buttonShape).padding(horizontal = 8.dp, vertical = 4.dp), color = l.text, fontSize = 13.sp, maxLines = 1)
        }
        if (open) {
            options.forEachIndexed { i, (name, on) ->
                Row(
                    Modifier.fillMaxWidth().height(36.dp).clip(l.buttonShape).background(if (on) l.accent.copy(alpha = 0.15f) else Color.Transparent)
                        .clickable { onPick(i); open = false }.padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) { Text(name, color = if (on) l.text else l.muted, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
        }
    }
}


/**
 * The orientation outside the player. A phone's screens are laid out
 * upright, so a phone goes back upright by request instead of being left
 * to the system (with rotation locked it could stay on its side after a
 * video); a tablet follows the system.
 */
fun restOrientation(activity: android.app.Activity?): Int =
    if ((activity?.resources?.configuration?.smallestScreenWidthDp ?: 0) < 600) android.content.pm.ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
    else android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
