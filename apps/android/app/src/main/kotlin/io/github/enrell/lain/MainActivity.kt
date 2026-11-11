package io.github.enrell.lain

import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.togetherWith
import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import io.github.enrell.lain.core.Item
import io.github.enrell.lain.download.DownloadService
import io.github.enrell.lain.ui.AppViewModel
import io.github.enrell.lain.ui.Dest
import io.github.enrell.lain.ui.Phase
import io.github.enrell.lain.ui.Screen
import io.github.enrell.lain.ui.screens.ConnectScreen
import io.github.enrell.lain.ui.screens.Dot
import io.github.enrell.lain.ui.screens.DownloadsScreen
import io.github.enrell.lain.ui.screens.HomeScreen
import io.github.enrell.lain.ui.screens.Kbd
import io.github.enrell.lain.ui.screens.Label
import io.github.enrell.lain.ui.screens.LibraryScreen
import io.github.enrell.lain.ui.screens.LoginScreen
import io.github.enrell.lain.ui.screens.PlayerScreen
import io.github.enrell.lain.ui.screens.ReaderScreen
import io.github.enrell.lain.ui.screens.SearchScreen
import io.github.enrell.lain.ui.screens.SettingsScreen
import io.github.enrell.lain.ui.screens.TitleScreen
import io.github.enrell.lain.ui.screens.episodeLabel
import io.github.enrell.lain.ui.theme.LainTheme
import io.github.enrell.lain.ui.theme.LocalLain
import io.github.enrell.lain.ui.theme.WebPalette
import io.github.enrell.lain.ui.screens.Ic
import io.github.enrell.lain.ui.screens.LibraryDetailScreen
import io.github.enrell.lain.ui.screens.Mono
import io.github.enrell.lain.ui.screens.Sheet
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // The app is dark whatever the system's mode: light icons on both
        // bars and no light scrim behind the navigation buttons.
        val dark = androidx.activity.SystemBarStyle.dark(android.graphics.Color.TRANSPARENT)
        enableEdgeToEdge(statusBarStyle = dark, navigationBarStyle = dark)
        requestedOrientation = io.github.enrell.lain.ui.screens.restOrientation(this)
        val graph = this.graph
        setContent {
            var themeId by remember { mutableStateOf(graph.prefs.theme) }
            val tokens = remember(themeId) { graph.themes.load(themeId) }
            // The server's palette, as the web client paints (and the one
            // kept from last time until it answers).
            var paletteJson by remember { mutableStateOf(graph.prefs.palette) }
            val palette = remember(paletteJson) { WebPalette.parse(paletteJson) ?: WebPalette.Default }
            LaunchedEffect(Unit) {
                while (true) {
                    graph.prefs.server?.let { server ->
                        WebPalette.fetch(server)?.let { json ->
                            if (json != paletteJson) {
                                graph.prefs.palette = json
                                paletteJson = json
                            }
                        }
                    }
                    delay(30_000)
                }
            }
            LainTheme(tokens, palette) {
                val vm: AppViewModel = viewModel { AppViewModel(graph.core, rememberServer = { graph.prefs.server = it }) }
                LaunchedEffect(Unit) { if (vm.state.value.phase == Phase.Starting) vm.start(graph.prefs.server) }
                Root(graph, vm, themeId) {
                    themeId = it
                    graph.prefs.theme = it
                }
            }
        }
    }

    fun needsNotificationAsk(): Boolean =
        Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED

    fun askNotifications() {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
    }
}

private fun labelOf(id: String): Int = when (id) {
    "home" -> R.string.nav_home
    "library" -> R.string.nav_library
    "read" -> R.string.nav_read
    "search" -> R.string.nav_search
    "downloads" -> R.string.nav_downloads
    else -> R.string.nav_settings
}

/**
 * The shell: a quiet top navigation built from `/api/modules` (D-250)
 * and the current screen. Keys: Ctrl+1..9 destinations, `/` search, Esc back.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Root(graph: AppGraph, vm: AppViewModel, themeId: String, onTheme: (String) -> Unit) {
    val l = LocalLain.current
    val state by vm.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val activity = context as? MainActivity
    var depth by remember { mutableStateOf(1) }
    val canBack = state.phase == Phase.Main && (state.stack.size > 1 || state.screen != Screen.Home || state.returnTo != null)
    BackHandler(enabled = canBack) { vm.back() }

    // The first download asks about notifications, with the reason, before it starts.
    var askFor by remember { mutableStateOf<Item?>(null) }
    val start: (Item) -> Unit = { item ->
        scope.launch {
            vm.download(item.id).fold(
                onSuccess = {
                    DownloadService.start(context)
                    val what = episodeLabel(item.season, item.episode, item.kind in io.github.enrell.lain.domain.READING_KINDS)
                    Toast.makeText(context, if (what.isEmpty()) context.getString(R.string.download_queued, item.title) else context.getString(R.string.download_queued_ep, item.title, what), Toast.LENGTH_SHORT).show()
                },
                onFailure = { Toast.makeText(context, it.message ?: it.toString(), Toast.LENGTH_LONG).show() },
            )
        }
    }

    val download: (Item) -> Unit = { item ->
        if (activity?.needsNotificationAsk() == true && !graph.prefs.notifyAsked) askFor = item else start(item)
    }
    askFor?.let { item ->
        val go = { allow: Boolean ->
            graph.prefs.notifyAsked = true
            askFor = null
            if (allow) activity?.askNotifications()
            start(item)
        }
        io.github.enrell.lain.ui.screens.Sheet(stringResource(R.string.notify_title), { go(false) }, footer = {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                io.github.enrell.lain.ui.screens.WButton(stringResource(R.string.notify_later), { go(false) }, Modifier.weight(1f), kind = io.github.enrell.lain.ui.screens.Btn.Secondary)
                io.github.enrell.lain.ui.screens.WButton(stringResource(R.string.notify_allow), { go(true) }, Modifier.weight(1.4f))
            }
        }) {
            Text(stringResource(R.string.notify_body), Modifier.padding(horizontal = 12.dp, vertical = 8.dp), color = l.muted, fontSize = 14.sp, lineHeight = 21.sp)
        }
    }

    Box(
        Modifier
            .fillMaxSize()
            .background(l.bg)
            .onPreviewKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown || state.phase != Phase.Main) return@onPreviewKeyEvent false
                val typing = state.screen == Screen.Search
                when {
                    e.key == Key.Escape -> vm.back()
                    e.isCtrlPressed && e.key in DIGITS -> {
                        state.destinations.getOrNull(DIGITS.indexOf(e.key))?.let { vm.go(it.screen) }
                        true
                    }
                    e.key == Key.Slash && !typing -> {
                        vm.go(Screen.Search)
                        true
                    }
                    else -> false
                }
            },
    ) {
        when (state.phase) {
            Phase.Starting -> Unit
            Phase.Connect -> Box(Modifier.safeDrawingPadding()) { ConnectScreen(state, vm) }
            Phase.Login -> Box(Modifier.safeDrawingPadding()) { LoginScreen(state, vm) }
            Phase.Main -> when (val s = state.screen) {
                is Screen.Player -> {
                    val item = state.items[s.itemId]
                    val title = item?.let { it.title + episodeLabel(it.season, it.episode, false).let { e -> if (e.isEmpty()) "" else " · $e" } } ?: ""
                    // What comes after it in its title, so the end of an episode leads on.
                    val nextItem = item?.let { cur ->
                        state.titles.firstOrNull { t -> t.items.any { it.id == cur.id } }?.items?.let { all -> all.getOrNull(all.indexOfFirst { it.id == cur.id } + 1) }
                    }
                    PlayerScreen(graph.core, s.itemId, title, state.user?.preferredLanguage ?: "", nextItem) { n -> vm.replace(Screen.Player(n.id)) }
                }
                is Screen.Reader -> {
                    val item = state.items[s.itemId]
                    val title = item?.let { it.title + episodeLabel(it.season, it.episode, true).let { e -> if (e.isEmpty()) "" else " · $e" } } ?: ""
                    val nextVolume = item?.let { cur ->
                        state.titles.firstOrNull { t -> t.items.any { it.id == cur.id } }?.items?.let { all -> all.getOrNull(all.indexOfFirst { it.id == cur.id } + 1) }
                    }
                    ReaderScreen(graph.core, s.itemId, title, { vm.back() }, nextVolume) { n -> vm.replace(Screen.Reader(n.id)) }
                }
                else -> Column(Modifier.fillMaxSize().safeDrawingPadding()) {
                    // A title page runs its art up under the status bar's edge
                    // and brings its own way back.
                    if (s !is Screen.TitleDetail) TopBar(state, vm)
                    // Screens cross-fade with a short rise; going back falls instead.
                    Box(Modifier.weight(1f)) {
                    AnimatedContent(
                        targetState = s,
                        modifier = Modifier.fillMaxSize(),
                        transitionSpec = {
                            val forward = state.stack.size >= depth
                            depth = state.stack.size
                            (fadeIn(tween(180)) + slideInVertically(tween(220)) { if (forward) it / 24 else -it / 24 }) togetherWith fadeOut(tween(120))
                        },
                        label = "screen",
                    ) { screen ->
                        when (screen) {
                            Screen.Home -> HomeScreen(state, vm)
                            Screen.Library -> LibraryScreen(state, vm, readingOnly = false)
                            Screen.Read -> LibraryScreen(state, vm, readingOnly = true)
                            Screen.Search -> SearchScreen(state, vm)
                            Screen.Downloads -> DownloadsScreen(graph, vm)
                            Screen.Settings -> SettingsScreen(graph, state, vm, themeId, onTheme)
                            is Screen.LibraryDetail -> LibraryDetailScreen(state, vm, screen.id)
                            is Screen.TitleDetail -> TitleScreen(state, vm, screen.key, download)
                            else -> Unit
                        }
                    }
                    // A row half out of view reads as "more below", not as cut by the bar.
                    Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(14.dp).background(Brush.verticalGradient(listOf(Color.Transparent, l.bg))))
                    if (s is Screen.TitleDetail) {
                        Box(Modifier.align(Alignment.TopCenter).fillMaxWidth().height(20.dp).background(Brush.verticalGradient(listOf(l.bg, Color.Transparent))))
                    }
                    }
                    // While typing, the keyboard takes the bar's place: the results get the room.
                    if (!WindowInsets.isImeVisible) BottomBar(state.destinations, s, vm)
                }
            }
        }
    }
}

private val DIGITS = listOf(Key.One, Key.Two, Key.Three, Key.Four, Key.Five, Key.Six, Key.Seven, Key.Eight, Key.Nine)

/** The slim top bar: the mark and name, the offline note, the account's initial. */
@Composable
private fun TopBar(state: io.github.enrell.lain.ui.AppState, vm: AppViewModel) {
    val l = LocalLain.current
    Column {
        Row(Modifier.fillMaxWidth().height(48.dp).padding(start = 20.dp, end = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Ic.Mark, null, tint = l.accent, modifier = Modifier.size(18.dp))
            Text("lain", Modifier.padding(start = 8.dp), color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 3.sp)
            Box(Modifier.weight(1f))
            if (!state.online) {
                Row(Modifier.padding(end = 10.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Dot(l.warn)
                    Mono(stringResource(R.string.offline), color = l.warn)
                }
            }
            Box(
                Modifier.size(34.dp).clip(CircleShape).background(l.raised).clickable { vm.go(Screen.Settings) },
                contentAlignment = Alignment.Center,
            ) { Text((state.user?.username ?: "?").take(1).uppercase(), color = l.accent, fontSize = 14.sp, fontWeight = FontWeight.SemiBold) }
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(l.line))
    }
}

private fun iconOf(id: String) = when (id) {
    "home" -> Ic.Home
    "library" -> Ic.Grid
    "search" -> Ic.Search
    "downloads" -> Ic.Download
    else -> Ic.Gear
}

/**
 * The bar: the same five places on every screen. Comics are libraries,
 * reached like the others, so nothing hides behind a "More".
 */
private val BAR = listOf("home", "library", "search", "downloads", "settings")

@Composable
private fun BottomBar(dests: List<Dest>, current: Screen, vm: AppViewModel) {
    val l = LocalLain.current
    val bar = BAR.mapNotNull { id -> dests.firstOrNull { it.id == id } }
    // A screen opened from a destination keeps that destination lit.
    val root = when (current) {
        is Screen.LibraryDetail, is Screen.TitleDetail, Screen.Read -> Screen.Library
        else -> current
    }
    Column(Modifier.background(l.bg)) {
        Box(Modifier.fillMaxWidth().height(1.dp).background(l.line))
        Row(Modifier.fillMaxWidth().height(62.dp)) {
            for (d in bar) Tab(stringResource(labelOf(d.id)), iconOf(d.id), d.screen == root, Modifier.weight(1f)) {
                // A destination opened from the bar starts at its top; Back is what returns to a place.
                if (d.screen == Screen.Library) io.github.enrell.lain.ui.screens.ListMemory.forget("library-root")
                vm.go(d.screen)
            }
        }
    }
}

@Composable
private fun Tab(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, on: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val l = LocalLain.current
    // The place you are in: a lighter pill and the accent, not the accent alone.
    val tint = if (on) l.accent else l.muted
    Column(
        modifier.fillMaxHeight().clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClickLabel = label, onClick = onClick)
            .semantics { selected = on },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Box(Modifier.width(58.dp).height(30.dp).clip(CircleShape).background(if (on) l.raised else Color.Transparent), contentAlignment = Alignment.Center) {
            Icon(icon, null, tint = tint, modifier = Modifier.size(22.dp))
        }
        Text(label, Modifier.padding(top = 5.dp), color = if (on) l.text else l.muted, fontSize = 11.sp, fontWeight = if (on) FontWeight.SemiBold else FontWeight.Medium, maxLines = 1)
    }
}
