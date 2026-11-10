package io.github.enrell.lain.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import io.github.enrell.lain.AppGraph
import io.github.enrell.lain.R
import io.github.enrell.lain.core.CopyState
import io.github.enrell.lain.download.DownloadService
import io.github.enrell.lain.ui.AppState
import io.github.enrell.lain.ui.AppViewModel
import io.github.enrell.lain.ui.DownloadsModel
import io.github.enrell.lain.ui.theme.LocalLain
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Offline copies of this server: progress, pause/resume, delete, and the device budget. */
@Composable
fun DownloadsScreen(graph: AppGraph, vm: AppViewModel) {
    val l = LocalLain.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val model = remember { DownloadsModel(graph.core, scope) }
    val state by model.state.collectAsState()
    val live by graph.downloads.state.collectAsState()
    LaunchedEffect(live.running) {
        while (true) {
            model.refresh()
            delay(if (live.running) 1_000 else 5_000)
        }
    }
    Column(Modifier.fillMaxSize().padding(l.sp(4)), verticalArrangement = Arrangement.spacedBy(l.sp(3))) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(l.sp(2))) {
            Text(stringResource(R.string.nav_downloads), style = MaterialTheme.typography.headlineLarge)
            state.usage?.let { u ->
                Label(
                    if (u.maxBytes > 0) stringResource(R.string.downloads_usage, bytes(u.usedBytes), bytes(u.maxBytes))
                    else bytes(u.usedBytes),
                )
            }
        }
        Text(stringResource(R.string.downloads_hint), color = l.muted, style = MaterialTheme.typography.bodySmall)
        Row(horizontalArrangement = Arrangement.spacedBy(l.sp(2))) {
            if (live.running) {
                Action(stringResource(R.string.downloads_stop), null, { DownloadService.stop(context) })
            } else {
                Action(stringResource(R.string.downloads_start), null, { DownloadService.start(context) }, primary = state.copies.any { it.state == CopyState.QUEUED })
            }
            Action(stringResource(R.string.action_refresh), null, { model.refresh(); vm.refreshProgress() })
        }
        live.stopped?.let { Text(stringResource(R.string.downloads_stopped, it), color = l.warn, style = MaterialTheme.typography.bodySmall) }
        live.evicted.lastOrNull()?.let { Text(stringResource(R.string.downloads_evicted, it), color = l.muted, style = MaterialTheme.typography.bodySmall) }
        ErrorLine(state.error)
        if (state.copies.isEmpty()) {
            Text(stringResource(R.string.downloads_empty), color = l.muted)
        }
        LazyColumn(verticalArrangement = Arrangement.spacedBy(l.sp(2)), modifier = Modifier.weight(1f)) {
            items(state.copies, key = { it.itemId }) { c ->
                val (done, total) = live.bytes[c.itemId] ?: (c.bytes to c.size)
                Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(l.sp(1))) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(l.sp(2))) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                c.title + episodeLabel(c.season, c.episode, c.kind in io.github.enrell.lain.domain.READING_KINDS).let { if (it.isEmpty()) "" else " · $it" },
                                style = MaterialTheme.typography.titleSmall,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Label(
                                when (c.state) {
                                    CopyState.DONE -> stringResource(R.string.copy_done, bytes(c.bytes))
                                    CopyState.QUEUED -> stringResource(R.string.copy_queued, bytes(done), if (total > 0) bytes(total) else "?")
                                    CopyState.PAUSED -> stringResource(R.string.copy_paused, bytes(done))
                                    CopyState.FAILED -> stringResource(R.string.copy_failed, c.error.ifBlank { c.code })
                                },
                                color = when (c.state) {
                                    CopyState.DONE -> l.ok
                                    CopyState.FAILED -> l.danger
                                    else -> l.muted
                                },
                            )
                        }
                        val item = vm.state.value.items[c.itemId]
                        if (c.state == CopyState.DONE && item != null) Action(stringResource(R.string.action_open), null, { vm.open(item) })
                        when (c.state) {
                            CopyState.QUEUED -> Action(stringResource(R.string.action_pause), null, { model.pause(c.itemId) })
                            CopyState.PAUSED, CopyState.FAILED -> Action(stringResource(R.string.action_resume), null, {
                                model.resume(c.itemId)
                                DownloadService.start(context)
                            })
                            CopyState.DONE -> Unit
                        }
                        Action(stringResource(R.string.action_delete), null, { model.remove(c.itemId) })
                    }
                    if (c.state == CopyState.QUEUED && total > 0) {
                        LinearProgressIndicator(progress = { (done.toFloat() / total).coerceIn(0f, 1f) }, modifier = Modifier.fillMaxWidth(), color = l.accent)
                    }
                }
            }
        }
    }
}

/** Settings (D-087 shape: label + hint left, control right; personal settings apply at once). */
@Composable
fun SettingsScreen(graph: AppGraph, state: AppState, vm: AppViewModel, themeId: String, onTheme: (String) -> Unit) {
    val l = LocalLain.current
    val scope = rememberCoroutineScope()
    var queued by remember { mutableLongStateOf(0L) }
    var policy by remember { mutableStateOf(graph.core.offlinePolicy()) }
    var budget by remember { mutableStateOf((policy.maxBytes shr 30).toString()) }
    var floor by remember { mutableStateOf((policy.minFreeBytes shr 30).toString()) }
    LaunchedEffect(Unit) { queued = vm.queuedProgress() }
    val themes = remember { graph.themes.list() }
    LazyColumn(Modifier.fillMaxSize().padding(l.sp(4)), verticalArrangement = Arrangement.spacedBy(l.sp(3))) {
        item { Text(stringResource(R.string.nav_settings), style = MaterialTheme.typography.headlineLarge) }
        item { Label(stringResource(R.string.settings_you)) }
        item {
            SettingRow(stringResource(R.string.settings_account), state.user?.let { "${it.username} · ${it.role}" } ?: "") {
                Action(stringResource(R.string.settings_logout), null, { vm.logout() })
            }
        }
        item {
            SettingRow(stringResource(R.string.settings_server), listOf(state.server, state.serverVersion).filter { it.isNotEmpty() }.joinToString(" · ")) {
                Action(stringResource(R.string.login_other_server), null, { vm.changeServer() })
            }
        }
        item {
            SettingRow(stringResource(R.string.settings_sync), stringResource(R.string.settings_sync_hint, queued)) {
                Action(stringResource(R.string.settings_sync_now), null, {
                    scope.launch {
                        runCatching { graph.core.flushProgress() }
                        queued = vm.queuedProgress()
                        vm.refreshProgress()
                    }
                })
            }
        }
        item { Label(stringResource(R.string.settings_appearance)) }
        items(themes, key = { it.id }) { t ->
            SettingRow(t.name, t.problem ?: "${t.id} · ${t.scheme}") {
                Action(
                    stringResource(if (t.id == themeId) R.string.settings_theme_active else R.string.settings_theme_use),
                    null,
                    { onTheme(t.id) },
                    primary = t.id == themeId,
                )
            }
        }
        item { Label(stringResource(R.string.settings_offline)) }
        item {
            SettingRow(stringResource(R.string.settings_budget), stringResource(R.string.settings_budget_hint)) {
                OutlinedTextField(budget, { v ->
                    budget = v.filter(Char::isDigit).take(5)
                    budget.toLongOrNull()?.let { g -> policy = policy.copy(maxBytes = g shl 30); graph.savePolicy(policy) }
                }, singleLine = true, modifier = Modifier.padding(start = l.sp(2)).fillMaxWidth(0.3f))
            }
        }
        item {
            SettingRow(stringResource(R.string.settings_floor), stringResource(R.string.settings_floor_hint)) {
                OutlinedTextField(floor, { v ->
                    floor = v.filter(Char::isDigit).take(4)
                    floor.toLongOrNull()?.let { g -> policy = policy.copy(minFreeBytes = g shl 30); graph.savePolicy(policy) }
                }, singleLine = true, modifier = Modifier.padding(start = l.sp(2)).fillMaxWidth(0.3f))
            }
        }
        item {
            SettingRow(stringResource(R.string.settings_evict), stringResource(R.string.settings_evict_hint)) {
                Switch(policy.evictWatched, { on -> policy = policy.copy(evictWatched = on); graph.savePolicy(policy) })
            }
        }
        item { Label(stringResource(R.string.settings_about, io.github.enrell.lain.core.coreVersion())) }
    }
}

@Composable
private fun SettingRow(label: String, hint: String, control: @Composable () -> Unit) {
    val l = LocalLain.current
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(label, style = MaterialTheme.typography.titleSmall)
            if (hint.isNotEmpty()) Text(hint, color = l.muted, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        control()
    }
}
