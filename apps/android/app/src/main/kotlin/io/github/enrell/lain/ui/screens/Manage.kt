package io.github.enrell.lain.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.enrell.lain.ui.Screen
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


/** Offline copies: what they take, what is queued, and each one with its state. */
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
    val queued = state.copies.count { it.state == CopyState.QUEUED }
    var removing by remember { mutableStateOf<String?>(null) }
    removing?.let { id ->
        Sheet(stringResource(R.string.remove_download_q), { removing = null }, footer = {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                WButton(stringResource(R.string.player_cancel), { removing = null }, Modifier.weight(1f), kind = Btn.Secondary)
                WButton(stringResource(R.string.remove_download), { model.remove(id); removing = null }, Modifier.weight(1f), kind = Btn.Danger)
            }
        }) {
            Text(stringResource(R.string.remove_download_body), color = l.muted, fontSize = 14.sp, lineHeight = 20.sp)
        }
    }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 20.dp, end = 20.dp, top = 18.dp, bottom = 32.dp)) {
        item {
            Text(stringResource(R.string.nav_downloads), color = l.text, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.4).sp)
            Text(stringResource(R.string.downloads_hint), Modifier.padding(top = 4.dp), color = l.muted, fontSize = 14.sp, lineHeight = 20.sp)
            // What the copies take of the room they were given.
            state.usage?.let { u ->
                Column(Modifier.padding(top = 18.dp).fillMaxWidth().clip(l.panelShape).border(1.dp, l.line.copy(alpha = 0.7f), l.panelShape).background(l.surface.copy(alpha = 0.4f)).padding(16.dp)) {
                    Row(verticalAlignment = Alignment.Bottom) {
                        Text(bytes(u.usedBytes), color = l.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold)
                        if (u.maxBytes > 0) Text(stringResource(R.string.downloads_of, bytes(u.maxBytes)), Modifier.padding(start = 6.dp, bottom = 2.dp), color = l.muted, fontSize = 13.sp)
                        Box(Modifier.weight(1f))
                        Text(pluralStringResource(R.plurals.count_copies, state.copies.size, state.copies.size), color = l.muted, fontSize = 13.sp)
                    }
                    if (u.maxBytes > 0) {
                        Box(Modifier.padding(top = 12.dp).fillMaxWidth().height(4.dp).clip(CircleShape).background(l.raised)) {
                            Box(Modifier.fillMaxWidth((u.usedBytes.toFloat() / u.maxBytes).coerceIn(0f, 1f)).height(4.dp).clip(CircleShape).background(l.accent))
                        }
                    }
                }
            }
            if (queued > 0 || live.running) {
                Row(Modifier.padding(top = 14.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                    if (live.running) {
                        WButton(stringResource(R.string.downloads_stop), { DownloadService.stop(context) }, kind = Btn.Secondary, icon = Ic.Pause)
                    } else {
                        WButton(pluralStringResource(R.plurals.downloads_start_n, queued, queued), { DownloadService.start(context) }, icon = Ic.Download)
                    }
                }
            }
            live.stopped?.let { Text(stringResource(R.string.downloads_stopped, it), Modifier.padding(top = 10.dp), color = l.warn, fontSize = 13.sp) }
            live.evicted.lastOrNull()?.let { Text(stringResource(R.string.downloads_evicted, it), Modifier.padding(top = 6.dp), color = l.muted, fontSize = 13.sp) }
            ErrorLine(state.error, Modifier.padding(top = 8.dp))
            Gap(18.dp)
            if (state.copies.isEmpty()) {
                EmptyState(Ic.Download, stringResource(R.string.downloads_empty_title), stringResource(R.string.downloads_empty)) {
                    WButton(stringResource(R.string.nav_library), { vm.go(Screen.Library) }, kind = Btn.Secondary)
                }
            }
        }
        items(state.copies, key = { it.itemId }) { c ->
            val (done, total) = live.bytes[c.itemId] ?: (c.bytes to c.size)
            val item = vm.state.value.items[c.itemId]
            val reading = c.kind in io.github.enrell.lain.domain.READING_KINDS
            Row(
                Modifier.padding(vertical = 4.dp).fillMaxWidth().clip(l.cardShape).border(1.dp, l.line.copy(alpha = 0.6f), l.cardShape).background(l.surface.copy(alpha = 0.3f))
                    .then(if (c.state == CopyState.DONE && item != null) Modifier.clickable { vm.open(item) } else Modifier),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Box(Modifier.width(if (reading) 56.dp else 116.dp).aspectRatio(if (reading) 3f / 4f else 16f / 9f)) {
                    Art(c.itemId, Modifier.fillMaxSize())
                    if (c.state == CopyState.QUEUED && total > 0) {
                        Box(Modifier.align(Alignment.BottomStart).fillMaxWidth().height(4.dp).background(Color.Black.copy(alpha = 0.5f))) {
                            Box(Modifier.fillMaxWidth((done.toFloat() / total).coerceIn(0f, 1f)).height(4.dp).background(l.accent))
                        }
                    }
                }
                Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                    Text(c.title, color = l.text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    val ep = episodeLabel(c.season, c.episode, reading)
                    Text(
                        listOf(
                            ep,
                            when (c.state) {
                                CopyState.DONE -> stringResource(R.string.copy_done, bytes(c.bytes))
                                CopyState.QUEUED -> stringResource(R.string.copy_queued, bytes(done), if (total > 0) bytes(total) else "?")
                                CopyState.PAUSED -> stringResource(R.string.copy_paused, bytes(done))
                                CopyState.FAILED -> stringResource(R.string.copy_failed, c.error.ifBlank { c.code })
                            },
                        ).filter { it.isNotEmpty() }.joinToString(" · "),
                        Modifier.padding(top = 4.dp),
                        color = when (c.state) {
                            CopyState.DONE -> l.ok
                            CopyState.FAILED -> l.danger
                            else -> l.muted
                        },
                        fontSize = 12.sp, maxLines = 2, overflow = TextOverflow.Ellipsis,
                    )
                }
                when (c.state) {
                    CopyState.QUEUED -> IconBtn(Ic.Pause, stringResource(R.string.action_pause), { model.pause(c.itemId) })
                    CopyState.PAUSED, CopyState.FAILED -> IconBtn(Ic.Download, stringResource(R.string.action_resume), {
                        model.resume(c.itemId)
                        DownloadService.start(context)
                    })
                    CopyState.DONE -> Unit
                }
                IconBtn(Ic.Trash, stringResource(R.string.remove_download), { removing = c.itemId })
            }
        }
    }
}

/** Settings, in the web client's shape: a name and what it does on the left, the control on the right. */
@Composable
fun SettingsScreen(graph: AppGraph, state: AppState, vm: AppViewModel, themeId: String, onTheme: (String) -> Unit) {
    val l = LocalLain.current
    val scope = rememberCoroutineScope()
    var queued by remember { mutableLongStateOf(0L) }
    var policy by remember { mutableStateOf(graph.core.offlinePolicy()) }
    var budget by remember { mutableStateOf((policy.maxBytes shr 30).toString()) }
    var floor by remember { mutableStateOf((policy.minFreeBytes shr 30).toString()) }
    var skip by remember { mutableStateOf(graph.prefs.skipSec) }
    LaunchedEffect(Unit) { queued = vm.queuedProgress() }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 20.dp, end = 20.dp, top = 18.dp, bottom = 40.dp)) {
        Text(stringResource(R.string.nav_settings), color = l.text, fontSize = 22.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.4).sp)
        // Who is signed in, where.
        Row(Modifier.padding(top = 18.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(56.dp).clip(CircleShape).background(l.raised), contentAlignment = Alignment.Center) {
                Text((state.user?.username ?: "?").take(1).uppercase(), color = l.accent, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
            }
            Column(Modifier.padding(start = 14.dp)) {
                Text(state.user?.username ?: "", color = l.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                Text((state.user?.role ?: "").replaceFirstChar { it.uppercase() }, Modifier.padding(top = 2.dp), color = l.muted, fontSize = 13.sp)
            }
        }
        SettingsGroup(stringResource(R.string.settings_playback)) {
            SettingRow(stringResource(R.string.player_skip), stringResource(R.string.settings_skip_hint), below = true) {
                Segmented(io.github.enrell.lain.domain.SKIP_CHOICES.map { it to if (it == 0) stringResource(R.string.settings_auto) else "$it s" }, skip, { skip = it; graph.prefs.skipSec = it })
            }
        }
        SettingsGroup(stringResource(R.string.settings_offline)) {
            SettingRow(stringResource(R.string.settings_budget), stringResource(R.string.settings_budget_hint)) {
                Field(budget, { v ->
                    budget = v.filter(Char::isDigit).take(5)
                    budget.toLongOrNull()?.let { g -> policy = policy.copy(maxBytes = g shl 30); graph.savePolicy(policy) }
                }, "", Modifier.width(88.dp), keyboardType = KeyboardType.Number)
            }
            SettingRow(stringResource(R.string.settings_floor), stringResource(R.string.settings_floor_hint)) {
                Field(floor, { v ->
                    floor = v.filter(Char::isDigit).take(4)
                    floor.toLongOrNull()?.let { g -> policy = policy.copy(minFreeBytes = g shl 30); graph.savePolicy(policy) }
                }, "", Modifier.width(88.dp), keyboardType = KeyboardType.Number)
            }
            SettingRow(stringResource(R.string.settings_evict), stringResource(R.string.settings_evict_hint)) {
                Toggle(policy.evictWatched, { on -> policy = policy.copy(evictWatched = on); graph.savePolicy(policy) }, stringResource(R.string.settings_evict))
            }
        }
        SettingsGroup(stringResource(R.string.settings_account)) {
            SettingRow(stringResource(R.string.settings_sync), stringResource(R.string.settings_sync_hint, queued)) {
                WButton(stringResource(R.string.settings_sync_now), {
                    scope.launch {
                        runCatching { graph.core.flushProgress() }
                        queued = vm.queuedProgress()
                        vm.refreshProgress()
                    }
                }, kind = Btn.Secondary)
            }
            SettingRow(stringResource(R.string.settings_server), listOf(state.server, state.serverVersion).filter { it.isNotEmpty() }.joinToString(" · ")) {
                WButton(stringResource(R.string.settings_change), { vm.changeServer() }, kind = Btn.Secondary)
            }
            SettingRow(stringResource(R.string.settings_session), stringResource(R.string.settings_session_hint)) {
                WButton(stringResource(R.string.settings_logout), { vm.logout() }, kind = Btn.Ghost)
            }
        }
        Mono(stringResource(R.string.settings_about, io.github.enrell.lain.core.coreVersion()), Modifier.padding(top = 28.dp))
    }
}
