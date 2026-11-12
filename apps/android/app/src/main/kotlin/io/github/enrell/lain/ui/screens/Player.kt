package io.github.enrell.lain.ui.screens

import android.content.Context
import android.net.Uri
import androidx.annotation.OptIn
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
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
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.Tracks
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.ExoPlaybackException
import androidx.media3.exoplayer.ExoPlayer
import io.github.enrell.lain.R
import io.github.enrell.lain.core.Item
import io.github.enrell.lain.core.LainCoreInterface
import io.github.enrell.lain.core.PlaySource
import io.github.enrell.lain.core.SourceKind
import io.github.enrell.lain.core.UserInfo
import io.github.enrell.lain.domain.RememberedTracks
import io.github.enrell.lain.domain.SubtitleMode
import io.github.enrell.lain.domain.TextPick
import io.github.enrell.lain.domain.TrackInfo
import io.github.enrell.lain.domain.TrackPrefs
import io.github.enrell.lain.domain.decideText
import io.github.enrell.lain.domain.startTracks
import io.github.enrell.lain.graph
import io.github.enrell.lain.player.Anime4kEffect
import io.github.enrell.lain.player.DeviceCaps
import io.github.enrell.lain.player.VideoEffect
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

/** The account's track preferences (D-280), from the user the core answered. */
internal fun trackPrefsOf(user: UserInfo?): TrackPrefs =
    if (user == null) TrackPrefs() else TrackPrefs(user.preferredAudioLanguage, user.preferredSubtitleLanguage, SubtitleMode.of(user.preferredSubtitleMode))

/** What was chosen last time for this title: Japanese audio picked on episode 3 is still Japanese on episode 4. */
private fun rememberedOf(prefs: io.github.enrell.lain.Prefs, title: String) =
    RememberedTracks(prefs.trackLanguageOf(title, "audio")?.takeIf { it.isNotEmpty() }, prefs.trackLanguageOf(title, "text"))

/**
 * A player for one item, starting in the track languages this title and the
 * account ask for (D-280): audio and text apart, as close to the rule as
 * Media3's own preferences go; the subtitles are settled once the tracks
 * are known ([settleText]).
 */
private fun newPlayer(context: Context, title: String, tracks: TrackPrefs): ExoPlayer =
    ExoPlayer.Builder(context).build().apply {
        val start = startTracks(tracks, rememberedOf(context.graph.prefs, title))
        trackSelectionParameters = trackSelectionParameters.buildUpon().apply {
            start.audioLanguage?.let { setPreferredAudioLanguage(it) }
            setPreferredTextLanguage(start.textLanguage)
            if (start.textDisabled) setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true)
            // Forced only: Media3 then takes the forced track in the audio's language.
            if (start.forcedOnly) setIgnoredTextSelectionFlags(C.SELECTION_FLAG_DEFAULT)
        }.build()
    }

private fun infoOf(f: androidx.media3.common.Format) =
    TrackInfo(f.language, forced = f.selectionFlags and C.SELECTION_FLAG_FORCED != 0, default = f.selectionFlags and C.SELECTION_FLAG_DEFAULT != 0)

/**
 * The account's subtitle rule over the tracks the file turned out to have
 * (D-280), which Media3's preferences cannot say alone: Auto shows the
 * subtitle language only when the audio that plays is not in it, and
 * Forced only takes a forced track in the subtitle language before the
 * audio's. Only tracks the device can render count.
 */
private fun settleText(player: ExoPlayer, tracks: Tracks, prefs: TrackPrefs, remembered: RememberedTracks, ui: PlayerUi) {
    val audioGroups = tracks.groups.filter { it.type == C.TRACK_TYPE_AUDIO }
    val texts = tracks.groups.filter { it.type == C.TRACK_TYPE_TEXT }.flatMap { g -> (0 until g.length).filter { g.isTrackSupported(it) }.map { g to it } }
    val audios = audioGroups.flatMap { g -> (0 until g.length).map { infoOf(g.getTrackFormat(it)) } }
    val playing = audioGroups.firstNotNullOfOrNull { g -> (0 until g.length).firstOrNull { g.isTrackSelected(it) }?.let { infoOf(g.getTrackFormat(it)) } }
    when (val pick = decideText(texts.map { (g, i) -> infoOf(g.getTrackFormat(i)) }, audios, playing, prefs, remembered)) {
        TextPick.Keep -> Unit
        TextPick.Off -> if (texts.any { (g, i) -> g.isTrackSelected(i) }) {
            ui.textOff = true
            player.trackSelectionParameters = player.trackSelectionParameters.buildUpon().setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true).build()
        }
        is TextPick.Track -> {
            val (g, i) = texts[pick.index]
            if (!g.isTrackSelected(i)) {
                ui.textOff = false
                player.trackSelectionParameters = player.trackSelectionParameters.buildUpon().setTrackTypeDisabled(C.TRACK_TYPE_TEXT, false)
                    .setOverrideForType(androidx.media3.common.TrackSelectionOverride(g.mediaTrackGroup, i)).build()
            }
        }
    }
}

/**
 * The player, with the web player's chrome and gestures (D-246 for the
 * decoding): a header with the way back and the title; a seek bar that
 * shows the frame it points at while dragged; play, 10 s back and forward,
 * mute, settings (speed, subtitles) and fullscreen underneath. A double
 * tap on the left or right third seeks 10 s and keeps counting while the
 * taps continue, without bringing the controls up; one tap shows or hides
 * them. Keys: Space, ←/→, B burns typeset subtitles in, Esc closes.
 *
 * This function owns the playback and its effects and stacks the chrome's
 * parts (PlayerChrome.kt, PlayerBar.kt, PlayerPanels.kt), which share a
 * [PlayerUi] and act through [PlayerMoves]. Add to a part, or make a new
 * one, rather than growing this function: as one function of about 800
 * lines it made the release build's PlayerKt fail to load.
 */
@OptIn(UnstableApi::class)
@Composable
fun PlayerScreen(
    core: LainCoreInterface,
    itemId: String,
    title: String,
    /** The account's audio and subtitle preferences (D-280). */
    trackPrefs: TrackPrefs,
    /** The item was already watched: playing it again keeps it so. */
    watchedBefore: Boolean = false,
    /** What plays after this one (the title's next episode), if anything. */
    next: Item? = null,
    /**
     * The name the audio and subtitle picks are kept under: the work's
     * title, so the next episode opens with them ([title] names the
     * episode too, which kept each pick to its own episode).
     */
    tracksKey: String = title,
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
    val stateRef = model.state.collectAsState()
    val state by stateRef
    remember(model) { model.rewatch = watchedBefore; Unit }
    val player = remember(itemId) { newPlayer(context, tracksKey, trackPrefs) }
    val focus = remember { FocusRequester() }
    val back = androidx.activity.compose.LocalOnBackPressedDispatcherOwner.current?.onBackPressedDispatcher
    val ui = remember { PlayerUi(graph.prefs) }
    val moves = remember(model, player) { PlayerMoves(model, player, ui, stateRef) }

    LaunchedEffect(player) {
        if (ui.speed != 1f) {
            player.setPlaybackSpeed(ui.speed)
            // Said once as it opens: a speed kept from another video is a surprise otherwise.
            ui.note.value = context.getString(R.string.player_speed_kept, "${ui.speed}×")
        }
    }

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
    // Picture in picture: the picture alone in a corner while something else is
    // on screen. Leaving the app while it plays goes there by itself; a video
    // nobody can see (the app stopped, the corner closed) pauses.
    val main = activity as? io.github.enrell.lain.MainActivity
    val canPip = remember { context.packageManager.hasSystemFeature(android.content.pm.PackageManager.FEATURE_PICTURE_IN_PICTURE) }
    val enterPip = {
        val v = player.videoSize
        val ratio = (if (v.width > 0 && v.height > 0) v.width.toFloat() / v.height else 16f / 9f).coerceIn(0.45f, 2.3f)
        ui.settings = false
        ui.tracksPanel = 0
        ui.controls = false
        // The corner is for watching: a paused picture sent there plays.
        if (!player.isPlaying) player.play()
        runCatching {
            activity?.enterPictureInPictureMode(
                android.app.PictureInPictureParams.Builder().setAspectRatio(android.util.Rational((ratio * 1000).toInt(), 1000)).build(),
            )
        }
        Unit
    }
    DisposableEffect(main) {
        val changed = androidx.core.util.Consumer<androidx.core.app.PictureInPictureModeChangedInfo> { ui.inPip = it.isInPictureInPictureMode }
        val seen = androidx.lifecycle.LifecycleEventObserver { _, e -> if (e == androidx.lifecycle.Lifecycle.Event.ON_STOP) player.pause() }
        main?.addOnPictureInPictureModeChangedListener(changed)
        main?.lifecycle?.addObserver(seen)
        onDispose {
            main?.removeOnPictureInPictureModeChangedListener(changed)
            main?.lifecycle?.removeObserver(seen)
            main?.leaveToPip = null
        }
    }
    androidx.compose.runtime.SideEffect { main?.leaveToPip = if (canPip && ui.playing) enterPip else null }
    LaunchedEffect(itemId) {
        // On mobile data, the quality chosen for it in Settings from the start.
        val metered = (context.getSystemService(android.content.Context.CONNECTIVITY_SERVICE) as? android.net.ConnectivityManager)?.isActiveNetworkMetered == true
        model.load(if (metered) graph.prefs.meteredQuality else "")
    }
    DisposableEffect(itemId) {
        val listener = object : Player.Listener {
            override fun onPlaybackStateChanged(playbackState: Int) {
                ui.buffering = playbackState == Player.STATE_BUFFERING
                ui.ended = playbackState == Player.STATE_ENDED
                if (ui.ended) model.report(player.currentPosition, player.duration, ended = true)
            }

            override fun onIsPlayingChanged(isPlaying: Boolean) {
                ui.playing = isPlaying
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
                // A track is named by its title and language; codec names and bare
                // language codes beside a full name are noise. A language offered twice
                // (in the file and again converted by the server) is listed once.
                fun of(type: Int, once: Boolean) = tracks.groups.filter { it.type == type }.flatMap { g ->
                    (0 until g.length).map { i ->
                        val f = g.getTrackFormat(i)
                        val lang = f.language?.takeIf { it != "und" }
                        val name = io.github.enrell.lain.domain.trackName(f.label, lang)
                        // Two audio tracks in one language are two tracks (a commentary); two subtitles are the same text
                        // twice, unless one is the forced (signs) track, which the account's rule tells apart (D-280).
                        val forced = f.selectionFlags and C.SELECTION_FLAG_FORCED != 0
                        (if (once) (lang ?: name) + if (forced) ":forced" else "" else "${g.hashCode()}:$i") to TextTrack(g, i, name.ifEmpty { "#${i + 1}" }, g.isTrackSelected(i))
                    }
                }.groupBy({ it.first }, { it.second }).values.map { same -> same.firstOrNull { it.selected } ?: same.first() }
                ui.textTracks = of(C.TRACK_TYPE_TEXT, once = true)
                ui.audioTracks = of(C.TRACK_TYPE_AUDIO, once = false)
                if (!ui.textDecided && !tracks.isEmpty) {
                    ui.textDecided = true
                    settleText(player, tracks, trackPrefs, rememberedOf(graph.prefs, tracksKey), ui)
                }
            }

            override fun onVideoSizeChanged(size: androidx.media3.common.VideoSize) {
                // A picture that is taller than wide is watched upright.
                if (size.width > 0 && size.height > size.width && ui.landscape) {
                    ui.landscape = false
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
        // A new source (a transcode after the decoder gave up) has its own tracks.
        ui.textDecided = false
        player.setMediaItem(mediaItem(src), model.seekOnStartMs(src))
        player.prepare()
        player.playWhenReady = true
        ui.pendingSec = null
        ui.positionMs = model.seekOnStartMs(src)
        focus.requestFocus()
        while (true) {
            delay(SAVE_EVERY_MS)
            if (player.isPlaying) model.report(player.currentPosition, player.duration)
        }
    }
    // The storyboard is asked for once the play is known; until it is
    // here the preview falls back to single frames.
    LaunchedEffect(itemId, src != null) {
        if (src != null && src.kind != SourceKind.OFFLINE && ui.board == null) ui.board = graph.storyboard(itemId)
    }
    // The video effect: Media3 runs the shaders on each decoded frame.
    LaunchedEffect(ui.effect, src != null) {
        if (src == null) return@LaunchedEffect
        if (ui.effect == VideoEffect.Off) {
            if (ui.effectsOn) player.setVideoEffects(emptyList())
            ui.effectsOn = false
        } else {
            val main = android.os.Handler(android.os.Looper.getMainLooper())
            player.setVideoEffects(
                listOf(
                    Anime4kEffect(ui.effect, { ui.shownAt.get() }) { reason ->
                        main.post {
                            ui.effectError = reason
                            ui.effect = VideoEffect.Off
                        }
                    },
                ),
            )
            ui.effectsOn = true
        }
    }
    // The clock the chrome reads: often enough for a smooth bar.
    LaunchedEffect(player) {
        while (true) {
            ui.positionMs = player.currentPosition
            ui.durationMs = player.duration.coerceAtLeast(0)
            ui.bufferedMs = player.bufferedPosition
            delay(250)
        }
    }
    // The controls leave after a while of playing untouched.
    // Paused or ended is a state you can see: the controls stay until it plays again.
    val idle = !ui.playing && !ui.buffering && !state.loading && state.source != null
    val pinned = idle || ui.settings || ui.tracksPanel != 0 || ui.scrub != null || state.error != null || state.loading || ui.countdown != null
    LaunchedEffect(ui.controls, ui.playing, pinned, ui.touched) {
        if (ui.controls && ui.playing && !pinned) {
            // Long enough to find a button and press it: a second and a half was a race.
            delay(l.tokens.autohideMs.toLong().coerceAtLeast(4500))
            ui.controls = false
            ui.settings = false
        }
    }
    LaunchedEffect(ui.flash) {
        if (ui.flash != null) {
            delay(900)
            ui.flash = null
        }
    }
    LaunchedEffect(ui.ended, next?.id) {
        if (ui.ended && next != null) {
            for (n in 6 downTo 1) {
                ui.countdown = n
                delay(1000)
                if (ui.countdown == null) return@LaunchedEffect
            }
            ui.countdown = null
            onNext(next)
        } else {
            ui.countdown = null
        }
    }
    LaunchedEffect(ui.hint, ui.playing) {
        if (ui.hint && ui.playing) {
            delay(4500)
            ui.hint = false
            graph.prefs.playerHintSeen = true
        }
    }

    // The parts in the order they stack, the picture at the bottom.
    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black)
            .onSizeChanged { ui.shownAt.set(it.width to it.height) }
            .focusRequester(focus)
            .focusable()
            .onPreviewKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when (e.key) {
                    Key.Spacebar, Key.MediaPlayPause -> { moves.togglePlay(); true }
                    Key.DirectionLeft -> { moves.seekBy(-moves.skipMs()); true }
                    Key.DirectionRight -> { moves.seekBy(moves.skipMs()); true }
                    Key.B -> {
                        model.burnable().firstOrNull()?.let { model.burnIn(it, player.currentPosition) }
                        true
                    }
                    else -> false
                }
            },
    ) {
        PlayerPicture(player, ui, pinned)
        // The picture's gestures (none in the corner: the system owns that window).
        if (!ui.inPip) PictureGestures(ui, moves)
        if (ui.buffering || state.loading) {
            CircularProgressIndicator(Modifier.align(Alignment.Center).size(44.dp), color = Color.White, strokeWidth = 3.dp)
        }
        val shown = (ui.controls || pinned) && !ui.inPip
        CentreTransport(ui, moves, shown, idle, state.loading)
        if (shown) PlayerTopBar(title, state) { back?.onBackPressed() }
        if (ui.hint && ui.playing && !pinned && !ui.inPip) GestureHint()
        NoteLine(ui.note, !ui.inPip, Modifier.align(Alignment.TopCenter))
        ui.countdown?.let { n ->
            if (next != null) NextCountdown(n, onCancel = { ui.countdown = null }, onPlay = { ui.countdown = null; onNext(next) })
        }
        if (shown) PlayerBar(ui, moves, model, state, itemId, player, next, onNext, canPip, enterPip)
        if (ui.settings || ui.tracksPanel != 0) PanelScrim(ui)
        if (ui.tracksPanel != 0) TracksPanel(ui, player, tracksKey)
        if (ui.settings) PlaybackPanel(ui, moves, model, state, player)
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
