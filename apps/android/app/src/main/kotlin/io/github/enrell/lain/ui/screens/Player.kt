package io.github.enrell.lain.ui.screens

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
fun PlayerScreen(core: LainCoreInterface, itemId: String, title: String, preferredLanguage: String) {
    val l = LocalLain.current
    val context = LocalContext.current
    val graph = context.graph
    val activity = context as? android.app.Activity
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

    var controls by remember { mutableStateOf(true) }
    var settings by remember { mutableStateOf(false) }
    var landscape by remember { mutableStateOf(false) }
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
        onDispose {
            bars?.show(androidx.core.view.WindowInsetsCompat.Type.systemBars())
            window?.clearFlags(android.view.WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            activity?.requestedOrientation = android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
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

            override fun onTracksChanged(tracks: Tracks) {
                textTracks = tracks.groups.filter { it.type == C.TRACK_TYPE_TEXT }.flatMap { g ->
                    (0 until g.length).map { i ->
                        val f = g.getTrackFormat(i)
                        TextTrack(g, i, f.label ?: f.language?.uppercase() ?: "#${i + 1}", g.isTrackSelected(i))
                    }
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
    val pinned = settings || scrub != null || state.error != null || state.loading
    LaunchedEffect(controls, playing, pinned, touched) {
        if (controls && playing && !pinned) {
            delay(l.tokens.autohideMs.toLong().coerceAtLeast(1500))
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
    fun skip(side: Int) {
        seekBy(side * SKIP_MS)
        flash = side to ((flash?.takeIf { it.first == side }?.second ?: 0) + (SKIP_MS / 1000).toInt())
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
            delay(700)
            flash = null
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
                    Key.DirectionLeft -> { seekBy(-SKIP_MS); true }
                    Key.DirectionRight -> { seekBy(SKIP_MS); true }
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

        if (buffering || state.loading) {
            CircularProgressIndicator(Modifier.align(Alignment.Center).size(44.dp), color = Color.White, strokeWidth = 3.dp)
        } else if (controls && (ended || (!playing && positionMs < 500))) {
            // The big button, before the first frame moves and at the end.
            Box(
                Modifier.align(Alignment.Center).size(72.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.55f)).clickable { togglePlay() },
                contentAlignment = Alignment.Center,
            ) { Icon(Ic.Play, null, tint = Color.White, modifier = Modifier.size(28.dp)) }
        }

        val shown = controls || pinned
        // Header: the way back and the title.
        if (shown) {
            Column(
                Modifier.align(Alignment.TopStart).fillMaxWidth()
                    .background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.6f), Color.Transparent))).safeDrawingPadding().padding(horizontal = 8.dp, vertical = 6.dp),
            ) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Row(
                        Modifier.height(44.dp).clip(l.buttonShape).clickable { back?.onBackPressed() }.padding(horizontal = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Icon(Ic.ArrowLeft, null, tint = Color.White, modifier = Modifier.size(20.dp))
                        Text(stringResource(R.string.player_back), color = Color.White, fontSize = 14.sp)
                    }
                    Text(title, Modifier.weight(1f).padding(start = 8.dp), color = Color.White, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Row(Modifier.padding(start = 16.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                    when {
                        state.loading -> Mono(stringResource(R.string.player_preparing), color = Color.White)
                        src != null && src.note.isNotBlank() -> Mono(src.note, color = Color.White.copy(alpha = 0.6f))
                    }
                    if (state.savedLocally) Mono(stringResource(R.string.player_saved_locally), color = l.warn)
                }
                ErrorLine(state.error, Modifier.padding(start = 16.dp, top = 6.dp))
            }
        }

        // Footer: the seek row, then the transport.
        if (shown) {
            val total = totalSec().coerceAtLeast(0.001)
            val at = scrub?.let { it * total } ?: nowSec()
            Column(
                Modifier.align(Alignment.BottomStart).fillMaxWidth()
                    .background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.7f)))).safeDrawingPadding().padding(horizontal = 12.dp).padding(bottom = 10.dp, top = 24.dp),
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
                                }.width(w),
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
                                Text(clock(sec), Modifier.padding(top = 4.dp), color = Color.White, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
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
                                        onDragStart = { o -> if (totalSec() > 0) scrub = (o.x / size.width).coerceIn(0f, 1f) },
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
                            Box(Modifier.fillMaxWidth().height(3.dp).background(Color.White.copy(alpha = 0.25f)))
                            // What is loaded sits where it is: a session that began
                            // partway holds a range inside the title, not its start.
                            val heldFrom = ((state.source?.offsetSec ?: 0.0) / total).toFloat().coerceIn(0f, 1f)
                            val heldTo = (model.sourceSec(bufferedMs) / total).toFloat().coerceIn(heldFrom, 1f)
                            Box(Modifier.offset(x = width * heldFrom).width(width * (heldTo - heldFrom)).height(3.dp).background(Color.White.copy(alpha = 0.3f)))
                            Box(Modifier.fillMaxWidth(fraction).height(3.dp).background(l.accent))
                            Box(Modifier.offset(x = (width - 14.dp) * fraction).size(if (scrub != null) 18.dp else 14.dp).clip(CircleShape).background(l.accent))
                        }
                    }
                    Text(
                        clock(at) + " / " + clock(totalSec()),
                        Modifier.padding(start = 12.dp),
                        color = Color.White, fontSize = 12.sp,
                    )
                }
                Row(Modifier.padding(top = 2.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    Box(
                        Modifier.size(44.dp).clip(l.buttonShape).background(l.accent).clickable { togglePlay() },
                        contentAlignment = Alignment.Center,
                    ) { Icon(if (playing) Ic.Pause else Ic.Play, null, tint = l.onAccent, modifier = Modifier.size(20.dp)) }
                    TenButton(Ic.Back10) { seekBy(-SKIP_MS); touched++ }
                    TenButton(Ic.Fwd10) { seekBy(SKIP_MS); touched++ }
                    IconBtn(if (muted) Ic.Muted else Ic.Volume, "Mute", { muted = !muted; player.volume = if (muted) 0f else 1f; touched++ }, tint = Color.White)
                    Box(Modifier.weight(1f))
                    IconBtn(Ic.Gear, "Settings", { settings = !settings }, tint = Color.White)
                    IconBtn(Ic.Maximize, "Fullscreen", {
                        landscape = !landscape
                        activity?.requestedOrientation =
                            if (landscape) android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE else android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
                        touched++
                    }, tint = Color.White)
                }
            }
        }

        // Settings: a small list above the gear, as on the web's phone layout.
        if (settings) {
            Column(
                Modifier.align(Alignment.BottomEnd).safeDrawingPadding().padding(end = 12.dp, bottom = 104.dp).width(264.dp).heightIn(max = 420.dp).clip(RoundedCornerShape(8.dp))
                    .verticalScroll(rememberScrollState()).background(l.bg).border(1.dp, l.line.copy(alpha = 0.6f), RoundedCornerShape(8.dp)).padding(horizontal = 12.dp, vertical = 6.dp),
            ) {
                PickRow(
                    stringResource(R.string.player_speed),
                    if (speed == 1f) stringResource(R.string.player_normal) else "$speed×",
                    SPEEDS.map { (if (it == 1f) stringResource(R.string.player_normal) else "$it×") to (it == speed) },
                ) { i ->
                    speed = SPEEDS[i]
                    player.setPlaybackSpeed(speed)
                }
                if (textTracks.isNotEmpty()) {
                    val off = stringResource(R.string.player_off)
                    PickRow(
                        stringResource(R.string.player_subtitles),
                        if (textOff) off else textTracks.firstOrNull { it.selected }?.label ?: off,
                        listOf(off to (textOff || textTracks.none { it.selected })) + textTracks.map { it.label to (!textOff && it.selected) },
                    ) { i ->
                        val b = player.trackSelectionParameters.buildUpon()
                        if (i == 0) {
                            textOff = true
                            b.setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true)
                        } else {
                            val t = textTracks[i - 1]
                            textOff = false
                            b.setTrackTypeDisabled(C.TRACK_TYPE_TEXT, false)
                                .setOverrideForType(TrackSelectionOverride(t.group.mediaTrackGroup, t.index))
                        }
                        player.trackSelectionParameters = b.build()
                    }
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

private const val SKIP_MS = 10_000L
private val SPEEDS = listOf(0.5f, 0.75f, 1f, 1.25f, 1.5f, 2f)

private class TextTrack(val group: Tracks.Group, val index: Int, val label: String, val selected: Boolean)

/** The 10 s buttons: the turning arrow with the number inside. */
@Composable
private fun TenButton(icon: androidx.compose.ui.graphics.vector.ImageVector, onClick: () -> Unit) {
    Box(Modifier.size(44.dp).clip(CircleShape).clickable(onClick = onClick), contentAlignment = Alignment.Center) {
        Icon(icon, null, tint = Color.White, modifier = Modifier.size(24.dp))
        Text("10", Modifier.padding(top = 2.dp), color = Color.White, fontSize = 8.sp, fontWeight = FontWeight.Bold)
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
