package io.github.enrell.lain.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.media3.common.C
import androidx.media3.common.TrackSelectionOverride
import androidx.media3.exoplayer.ExoPlayer
import io.github.enrell.lain.R
import io.github.enrell.lain.core.SourceKind
import io.github.enrell.lain.domain.SKIP_CHOICES
import io.github.enrell.lain.domain.skipSeconds
import io.github.enrell.lain.graph
import io.github.enrell.lain.player.VideoEffect
import io.github.enrell.lain.ui.PlayerModel
import io.github.enrell.lain.ui.PlayerState
import io.github.enrell.lain.ui.theme.LocalLain

private val SPEEDS = listOf(0.5f, 0.75f, 1f, 1.25f, 1.5f, 2f)

/** A list over the picture closes with a tap outside it. */
@Composable
internal fun PanelScrim(ui: PlayerUi) {
    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.55f)).pointerInput(Unit) { detectTapGestures { ui.settings = false; ui.tracksPanel = 0 } })
}

/** The subtitle or the audio list ([PlayerUi.tracksPanel]), above the bar; a pick is kept under [title] (the work's). */
@Composable
internal fun BoxScope.TracksPanel(ui: PlayerUi, player: ExoPlayer, title: String) {
    val l = LocalLain.current
    val context = LocalContext.current
    val graph = context.graph
    val subs = ui.tracksPanel == 1
    Column(
        Modifier.align(Alignment.BottomEnd).safeDrawingPadding().padding(end = 12.dp, bottom = 104.dp).width(264.dp).heightIn(max = 360.dp).clip(RoundedCornerShape(10.dp))
            .verticalScroll(rememberScrollState()).background(l.bg).border(1.dp, l.line.copy(alpha = 0.6f), RoundedCornerShape(10.dp)).padding(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(if (subs) R.string.player_subtitles else R.string.player_audio), Modifier.weight(1f).padding(start = 8.dp), color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
            IconBtn(Ic.Close, "Close", { ui.tracksPanel = 0 }, tint = l.text)
        }
        if (subs) {
            OptionRow(stringResource(R.string.player_off), null, ui.textOff || ui.textTracks.none { it.selected }, inset = 0.dp, onClick = {
                ui.textOff = true
                player.trackSelectionParameters = player.trackSelectionParameters.buildUpon().setTrackTypeDisabled(C.TRACK_TYPE_TEXT, true).build()
                graph.prefs.setTrackLanguageOf(title, "text", "")
                ui.tracksPanel = 0
            })
        }
        for (t in if (subs) ui.textTracks else ui.audioTracks) {
            OptionRow(t.label, null, t.selected && !(subs && ui.textOff), inset = 0.dp, onClick = {
                val type = if (subs) C.TRACK_TYPE_TEXT else C.TRACK_TYPE_AUDIO
                if (subs) ui.textOff = false
                player.trackSelectionParameters = player.trackSelectionParameters.buildUpon().setTrackTypeDisabled(type, false)
                    .setOverrideForType(TrackSelectionOverride(t.group.mediaTrackGroup, t.index)).build()
                // Kept for this title, so its next part opens with the same choice.
                t.group.getTrackFormat(t.index).language?.let { graph.prefs.setTrackLanguageOf(title, if (subs) "text" else "audio", it) }
                // Said, since nothing on the picture shows which track plays.
                ui.note.value = context.getString(if (subs) R.string.player_subtitles_now else R.string.player_audio_now, t.label)
                ui.tracksPanel = 0
            })
        }
    }
}

/** Playback, behind the gear: picture, speed, quality, skip step, effects, the source and the burn-in. */
@Composable
internal fun BoxScope.PlaybackPanel(ui: PlayerUi, moves: PlayerMoves, model: PlayerModel, state: PlayerState, player: ExoPlayer) {
    val l = LocalLain.current
    val graph = LocalContext.current.graph
    val src = state.source
    // Above the bar, as the track lists are: run to the bottom, the panel cut the bar's
    // labels in half ("Subtitles · Englis") and covered the seek line. It scrolls when
    // the screen is short.
    Column(
        Modifier.align(Alignment.BottomEnd).safeDrawingPadding().padding(end = 12.dp, top = 12.dp, bottom = 104.dp).width(300.dp).clip(RoundedCornerShape(10.dp))
            .verticalScroll(rememberScrollState()).background(l.bg).border(1.dp, l.line.copy(alpha = 0.6f), RoundedCornerShape(10.dp)).padding(horizontal = 14.dp, vertical = 8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.player_playback), Modifier.weight(1f), color = l.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
            IconBtn(Ic.Close, "Close", { ui.settings = false }, tint = l.text)
        }
        PickRow(
            stringResource(R.string.player_picture),
            stringResource(if (ui.fill) R.string.player_fill else R.string.player_fit),
            listOf(stringResource(R.string.player_fit) to !ui.fill, stringResource(R.string.player_fill) to ui.fill),
        ) { i -> ui.fill = i == 1 }
        PickRow(
            stringResource(R.string.player_speed),
            if (ui.speed == 1f) stringResource(R.string.player_normal) else "${ui.speed}×",
            SPEEDS.map { (if (it == 1f) stringResource(R.string.player_normal) else "$it×") to (it == ui.speed) },
        ) { i ->
            ui.speed = SPEEDS[i]
            graph.prefs.speed = ui.speed
            player.setPlaybackSpeed(ui.speed)
        }
        if (src != null && src.kind != SourceKind.OFFLINE && state.qualities.isNotEmpty()) {
            // The file as it is ("Original") while it plays directly; else the server's choice ("Auto").
            val direct = state.directFirst
            val auto = stringResource(if (direct) R.string.player_original_short else R.string.player_auto)
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
                    ui.pendingSec = moves.nowSec()
                    if (pick.isEmpty() && direct) model.original(moves.nowSec()) else model.rebuild(moves.nowSec(), quality = pick)
                }
            }
        }
        val byLength = stringResource(R.string.player_skip_auto, skipSeconds(0, moves.totalSec()))
        PickRow(
            stringResource(R.string.player_skip),
            if (ui.skipChoice == 0) byLength else "${ui.skipChoice} s",
            SKIP_CHOICES.map { (if (it == 0) byLength else "$it s") to (it == ui.skipChoice) },
        ) { i ->
            ui.skipChoice = SKIP_CHOICES[i]
            graph.prefs.skipSec = ui.skipChoice
        }
        PickRow(
            stringResource(R.string.player_effects),
            ui.effect.label,
            VideoEffect.entries.map { it.label to (it == ui.effect) },
        ) { i ->
            ui.effectError = null
            ui.effect = VideoEffect.entries[i]
            graph.prefs.videoEffect = ui.effect.id
        }
        ui.effectError?.let { Text(stringResource(R.string.player_effect_failed, it), color = l.danger, fontSize = 12.sp, lineHeight = 16.sp) }
        Text(stringResource(R.string.player_effects_hint), Modifier.padding(bottom = 4.dp), color = l.muted, fontSize = 12.sp, lineHeight = 16.sp)
        // How it is reaching you, as a fact with its name (not more help text under Upscaling).
        src?.note?.takeIf { it.isNotBlank() }?.let { note ->
            Row(Modifier.fillMaxWidth().height(40.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.player_source), Modifier.weight(1f), color = l.text, fontSize = 13.sp)
                Text(
                    if (note.contains("direct", ignoreCase = true)) stringResource(R.string.player_original) else stringResource(R.string.player_converted, note.lowercase()),
                    color = l.muted, fontSize = 13.sp,
                )
            }
        }
        val burnable = model.burnable()
        if (burnable.isNotEmpty()) {
            Text(stringResource(R.string.player_ass_note), Modifier.padding(top = 6.dp), color = l.muted, fontSize = 12.sp, lineHeight = 16.sp)
            WButton(
                stringResource(R.string.player_burn_in, burnable.first().label),
                { ui.settings = false; model.burnIn(burnable.first(), player.currentPosition) },
                Modifier.fillMaxWidth().padding(vertical = 6.dp),
                kind = Btn.Secondary,
            )
        }
    }
}

/** One setting: its name, the current value, and the choices when opened. */
@Composable
private fun PickRow(label: String, value: String, options: List<Pair<String, Boolean>>, onPick: (Int) -> Unit) {
    val l = LocalLain.current
    var open by remember { mutableStateOf(false) }
    Column {
        Row(Modifier.fillMaxWidth().height(40.dp).clickable(onClickLabel = label) { open = !open }, verticalAlignment = Alignment.CenterVertically) {
            Text(label, Modifier.weight(1f), color = l.text, fontSize = 13.sp)
            // The value with a chevron that turns: a bare chip did not say it opens a choice,
            // and read like the plain facts (Source) under it.
            Row(
                Modifier.clip(l.buttonShape).background(l.tonal).padding(start = 8.dp, end = 4.dp, top = 4.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(value, color = l.text, fontSize = 13.sp, maxLines = 1)
                Icon(Ic.ChevronDown, null, tint = l.muted, modifier = Modifier.size(16.dp).graphicsLayer { rotationZ = if (open) 180f else 0f })
            }
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
