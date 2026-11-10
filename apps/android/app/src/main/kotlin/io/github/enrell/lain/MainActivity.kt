package io.github.enrell.lain

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
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val graph = this.graph
        setContent {
            var themeId by remember { mutableStateOf(graph.prefs.theme) }
            val tokens = remember(themeId) { graph.themes.load(themeId) }
            LainTheme(tokens) {
                val vm: AppViewModel = viewModel { AppViewModel(graph.core, rememberServer = { graph.prefs.server = it }) }
                LaunchedEffect(Unit) { if (vm.state.value.phase == Phase.Starting) vm.start(graph.prefs.server) }
                Root(graph, vm, themeId) {
                    themeId = it
                    graph.prefs.theme = it
                }
            }
        }
    }

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
@Composable
private fun Root(graph: AppGraph, vm: AppViewModel, themeId: String, onTheme: (String) -> Unit) {
    val l = LocalLain.current
    val state by vm.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val activity = context as? MainActivity
    val canBack = state.phase == Phase.Main && (state.stack.size > 1 || state.screen != Screen.Home)
    BackHandler(enabled = canBack) { vm.back() }

    val download: (Item) -> Unit = { item ->
        scope.launch {
            vm.download(item.id).fold(
                onSuccess = {
                    activity?.askNotifications()
                    DownloadService.start(context)
                    Toast.makeText(context, context.getString(R.string.download_queued, item.title), Toast.LENGTH_SHORT).show()
                },
                onFailure = { Toast.makeText(context, it.message ?: it.toString(), Toast.LENGTH_LONG).show() },
            )
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
                    PlayerScreen(graph.core, s.itemId, title, state.user?.preferredLanguage ?: "")
                }
                is Screen.Reader -> Box(Modifier.safeDrawingPadding()) { ReaderScreen(graph.core, s.itemId) }
                else -> Column(Modifier.fillMaxSize().safeDrawingPadding()) {
                    NavBar(state.destinations, s, state.online, vm)
                    Box(Modifier.weight(1f)) {
                        when (s) {
                            Screen.Home -> HomeScreen(state, vm)
                            Screen.Library -> LibraryScreen(state, vm, readingOnly = false)
                            Screen.Read -> LibraryScreen(state, vm, readingOnly = true)
                            Screen.Search -> SearchScreen(state, vm)
                            Screen.Downloads -> DownloadsScreen(graph, vm)
                            Screen.Settings -> SettingsScreen(graph, state, vm, themeId, onTheme)
                            is Screen.TitleDetail -> TitleScreen(state, vm, s.key, download)
                            else -> Unit
                        }
                    }
                }
            }
        }
    }
}

private val DIGITS = listOf(Key.One, Key.Two, Key.Three, Key.Four, Key.Five, Key.Six, Key.Seven, Key.Eight, Key.Nine)

@Composable
private fun NavBar(dests: List<Dest>, current: Screen, online: Boolean, vm: AppViewModel) {
    val l = LocalLain.current
    Row(
        Modifier.fillMaxWidth().background(l.scrim).horizontalScroll(rememberScrollState()).padding(horizontal = l.sp(3), vertical = l.sp(2)),
        horizontalArrangement = Arrangement.spacedBy(l.sp(3)),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Label("lain", color = l.accent)
        dests.forEachIndexed { i, d ->
            val active = d.screen == current
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(l.sp(1))) {
                io.github.enrell.lain.ui.screens.Action(
                    stringResource(labelOf(d.id)),
                    null,
                    { vm.go(d.screen) },
                    primary = active,
                )
                if (i < 9) Kbd("^${i + 1}")
            }
        }
        if (!online) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(l.sp(1))) {
                Dot(l.warn)
                Text(stringResource(R.string.offline), style = l.label(l.warn))
            }
        }
    }
}
