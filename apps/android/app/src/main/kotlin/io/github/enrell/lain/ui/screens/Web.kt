package io.github.enrell.lain.ui.screens

import androidx.compose.ui.semantics.password
import androidx.compose.animation.core.animateFloat
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.focus.focusRequester
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.ui.layout.onSizeChanged
import kotlinx.coroutines.launch
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.enrell.lain.ui.theme.LocalLain

/*
 * The web client's phone interface, piece by piece: the same buttons,
 * cards, sheets and fields, at the same sizes, so the app and the site
 * read as one program.
 */

/** The shape of each title's picture, learned when it loads (web: shapes.svelte.ts). */
object Shapes {
    private val landscape = mutableStateMapOf<String, Boolean>()

    fun record(id: String, width: Int, height: Int) {
        if (width <= 0 || height <= 0) return
        val wide = width > height * 1.15f
        if (landscape[id] != wide) landscape[id] = wide
    }

    /** The measured shape, or [guess] until the picture has loaded. */
    fun landscape(id: String, guess: Boolean): Boolean = landscape[id] ?: guess

    const val PORTRAIT = 2f / 3f
    const val LANDSCAPE = 16f / 9f
}

enum class Btn { Primary, Secondary, Ghost, Danger }

@Composable
fun WButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    kind: Btn = Btn.Primary,
    icon: ImageVector? = null,
    large: Boolean = false,
    enabled: Boolean = true,
    pill: Boolean = false,
) {
    val l = LocalLain.current
    val shape = if (pill) CircleShape else l.buttonShape
    val (bg, fg, edge) = when (kind) {
        Btn.Primary -> Triple(l.accent, l.onAccent, Color.Transparent)
        Btn.Secondary -> Triple(l.raised, l.text, l.text.copy(alpha = 0.22f))
        Btn.Ghost -> Triple(Color.Transparent, l.muted, Color.Transparent)
        Btn.Danger -> Triple(l.danger.copy(alpha = 0.15f), l.danger, l.danger.copy(alpha = 0.3f))
    }
    // Disabled is a neutral surface, not a faded accent: on a muted theme
    // the two looked alike.
    val fill = if (enabled) bg else Color.Transparent
    val ink = if (enabled) fg else l.muted.copy(alpha = 0.45f)
    Row(
        modifier
            .height(if (large) 48.dp else 44.dp)
            .clip(shape)
            .background(fill)
            .border(1.dp, if (enabled) edge else l.line.copy(alpha = 0.5f), shape)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = if (large) 24.dp else 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
    ) {
        if (icon != null) Icon(icon, null, tint = ink, modifier = Modifier.size(16.dp))
        Text(text, color = ink, fontSize = if (large) 16.sp else 14.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

@Composable
fun IconBtn(icon: ImageVector, label: String, onClick: () -> Unit, modifier: Modifier = Modifier, tint: Color? = null, outlined: Boolean = false) {
    val l = LocalLain.current
    Box(
        modifier
            .size(44.dp)
            .clip(l.buttonShape)
            .then(if (outlined) Modifier.border(1.dp, l.line, l.buttonShape) else Modifier)
            .clickable(onClickLabel = label, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, label, tint = tint ?: l.muted, modifier = Modifier.size(20.dp)) }
}

/** A mono, uppercase, spaced label: the web's `font-mono text-[10px] tracking`. */
@Composable
fun Mono(text: String, modifier: Modifier = Modifier, color: Color? = null, size: Int = 10) {
    val l = LocalLain.current
    Text(
        text.uppercase(),
        modifier = modifier,
        // A label is small and spaced: it needs more light than body's secondary text to read.
        style = TextStyle(fontFamily = l.mono, fontSize = size.sp, letterSpacing = (size * 0.18f).sp, color = color ?: l.text.copy(alpha = 0.66f), fontWeight = FontWeight.Medium),
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
    )
}

/**
 * A card in the shape of its picture: a portrait cover or a landscape
 * still, with the title under it. [modifier] sets the width.
 */
@Composable
fun ShapeCard(
    artId: String,
    title: String,
    subtitle: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    landscape: Boolean,
    progress: Float = 0f,
    badge: String? = null,
) {
    val l = LocalLain.current
    Column(modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClick)) {
        Box(
            Modifier
                .fillMaxWidth()
                .aspectRatio(if (landscape) Shapes.LANDSCAPE else Shapes.PORTRAIT)
                .clip(l.cardShape)
                .background(l.surface)
                .border(1.dp, Color.White.copy(alpha = 0.05f), l.cardShape),
        ) {
            Art(artId, Modifier.fillMaxSize(), label = title)
            if (badge != null) {
                Text(
                    badge,
                    Modifier.padding(8.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.65f)).padding(horizontal = 8.dp, vertical = 2.dp),
                    style = TextStyle(fontFamily = l.mono, fontSize = 10.sp, fontWeight = FontWeight.SemiBold, color = Color.White),
                )
            }
            // Where you are in it: a bar while in progress, a check once it is done.
            if (progress >= 0.999f) {
                Box(Modifier.align(Alignment.TopEnd).padding(8.dp).size(24.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.65f)), contentAlignment = Alignment.Center) {
                    Icon(Ic.Check, null, tint = Color.White, modifier = Modifier.size(14.dp))
                }
            } else if (progress > 0f) {
                // Inset and rounded: flush with the edge it was half cut by the card's corners.
                Box(Modifier.align(Alignment.BottomStart).padding(horizontal = 10.dp, vertical = 9.dp).fillMaxWidth().height(5.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.6f))) {
                    Box(Modifier.fillMaxWidth(progress.coerceIn(0f, 1f)).height(5.dp).clip(CircleShape).background(l.accent))
                }
            }
        }
        Text(
            title,
            Modifier.padding(top = 10.dp),
            color = l.text, fontSize = 14.sp, lineHeight = 19.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis,
        )
        if (subtitle.isNotEmpty()) Text(subtitle, Modifier.padding(top = 3.dp), color = l.muted, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}

/** Every dialog is a bottom sheet on a phone. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun Sheet(
    title: String,
    onDismiss: () -> Unit,
    footer: (@Composable () -> Unit)? = null,
    /** False for a sheet that holds unsaved choices: scrolling it back up must not fling it shut. */
    dragToClose: Boolean = true,
    content: @Composable ColumnScope.() -> Unit,
) {
    val l = LocalLain.current
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        sheetGesturesEnabled = dragToClose,
        containerColor = l.surface,
        contentColor = l.text,
        scrimColor = Color.Black.copy(alpha = 0.62f),
        shape = RoundedCornerShape(topStart = 16.dp, topEnd = 16.dp),
        dragHandle = {
            Box(Modifier.padding(top = 12.dp, bottom = 4.dp).width(40.dp).height(4.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.15f)))
        },
    ) {
        Column(Modifier.fillMaxWidth().navigationBarsPadding()) {
            Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(title, Modifier.weight(1f), fontSize = 18.sp, fontWeight = FontWeight.SemiBold, color = l.text)
                IconBtn(Ic.Close, "Close", onDismiss)
            }
            Box(Modifier.weight(1f, fill = false)) {
                Column(
                    Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(bottom = if (footer != null) 28.dp else 20.dp),
                    content = content,
                )
                // What scrolls under the footer fades into it: a row half out of view reads as "more below", not as cut.
                if (footer != null) {
                    Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(24.dp).background(Brush.verticalGradient(listOf(Color.Transparent, l.surface))))
                }
            }
            if (footer != null) {
                // A ground of its own, so the list scrolls under an edge and not under the buttons.
                Box(Modifier.fillMaxWidth().height(1.dp).background(l.line.copy(alpha = 0.7f)))
                Box(Modifier.fillMaxWidth().background(l.surface).padding(horizontal = 20.dp, vertical = 12.dp)) { footer() }
            }
        }
    }
}

/** The web's search field: a quiet filled box with the glass on the left. */
@Composable
fun SearchField(value: String, onChange: (String) -> Unit, placeholder: String, modifier: Modifier = Modifier, focus: androidx.compose.ui.focus.FocusRequester? = null) {
    val l = LocalLain.current
    val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
    val focusManager = androidx.compose.ui.platform.LocalFocusManager.current
    // The field you are typing in wears the accent.
    var focused by remember { androidx.compose.runtime.mutableStateOf(false) }
    Row(
        modifier.height(44.dp).clip(l.buttonShape).background(l.field).border(if (focused) 1.5.dp else 1.dp, if (focused) l.accent else l.line, l.buttonShape).padding(start = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Ic.Search, null, tint = l.muted, modifier = Modifier.size(16.dp))
        Box(Modifier.weight(1f).padding(horizontal = 10.dp), contentAlignment = Alignment.CenterStart) {
            if (value.isEmpty()) Text(placeholder, color = l.muted.copy(alpha = 0.7f), fontSize = 16.sp, maxLines = 1)
            BasicTextField(
                value = value,
                onValueChange = onChange,
                singleLine = true,
                textStyle = TextStyle(color = l.text, fontSize = 16.sp),
                cursorBrush = SolidColor(l.accent),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                // The search key puts the keyboard away: what it covered is reachable again.
                keyboardActions = androidx.compose.foundation.text.KeyboardActions(onSearch = { keyboard?.hide(); focusManager.clearFocus() }),
                modifier = Modifier.fillMaxWidth().then(if (focus != null) Modifier.focusRequester(focus) else Modifier).onFocusChanged { focused = it.isFocused }.semantics { contentDescription = placeholder },
            )
        }
        if (value.isNotEmpty()) IconBtn(Ic.Close, "Clear", { onChange("") })
    }
}

/**
 * A filter option row: label, the count it would leave, and when it is on
 * a lighter surface, an accent bar and a check. (An accent wash alone
 * turned to mud on a muted theme and read as a statistic, not a choice.)
 */
@Composable
fun OptionRow(label: String, count: Int?, on: Boolean, onClick: () -> Unit, enabled: Boolean = true, multi: Boolean = false) {
    val l = LocalLain.current
    // A row just picked comes wholly into view, with room to spare: at the edge
    // of a sheet it was left half under the footer.
    val reveal = remember { androidx.compose.foundation.relocation.BringIntoViewRequester() }
    val scope = androidx.compose.runtime.rememberCoroutineScope()
    var size by remember { mutableStateOf(androidx.compose.ui.unit.IntSize.Zero) }
    val room = with(androidx.compose.ui.platform.LocalDensity.current) { 36.dp.toPx() }
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 46.dp)
            .onSizeChanged { size = it }
            .bringIntoViewRequester(reveal)
            .clip(l.buttonShape)
            .background(if (on) l.raised else Color.Transparent)
            .clickable(enabled = enabled || on) {
                onClick()
                scope.launch { reveal.bringIntoView(androidx.compose.ui.geometry.Rect(0f, -room, size.width.toFloat(), size.height + room)) }
            }
            .semantics { stateDescription = if (on) "selected" else "not selected" },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(3.dp).height(22.dp).clip(CircleShape).background(if (on) l.accent else Color.Transparent))
        val c = if (on) l.text else l.muted.copy(alpha = if (enabled) 1f else 0.35f)
        Text(label, Modifier.weight(1f).padding(start = 9.dp), color = c, fontSize = 14.sp, fontWeight = if (on) FontWeight.Medium else FontWeight.Normal, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (count != null) Text(count.toString(), style = TextStyle(fontFamily = l.mono, fontSize = 11.sp, color = c.copy(alpha = c.alpha * 0.7f)))
        // A square where several can be picked, a ring where one can: the mark says which.
        val mark = if (multi) RoundedCornerShape(5.dp) else CircleShape
        Box(
            Modifier.padding(start = 10.dp, end = 10.dp).size(20.dp).clip(mark).background(if (on) l.accent else Color.Transparent)
                .border(1.5.dp, if (on) l.accent else l.muted.copy(alpha = if (enabled) 0.55f else 0.2f), mark),
            contentAlignment = Alignment.Center,
        ) { if (on) Icon(Ic.Check, null, tint = l.onAccent, modifier = Modifier.size(12.dp)) }
    }
}

/** A choice among a few, as outlined tiles: the one that is on has the accent's outline and a check. */
@Composable
fun ChoiceTile(label: String, on: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val l = LocalLain.current
    Row(
        modifier
            .heightIn(min = 46.dp)
            .clip(l.buttonShape)
            .background(if (on) l.raised else Color.Transparent)
            .border(if (on) 1.5.dp else 1.dp, if (on) l.accent else l.line, l.buttonShape)
            .clickable(onClick = onClick)
            .padding(horizontal = 12.dp)
            .semantics { stateDescription = if (on) "selected" else "not selected" },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, Modifier.weight(1f), color = if (on) l.text else l.muted, fontSize = 14.sp, fontWeight = if (on) FontWeight.Medium else FontWeight.Normal, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (on) Icon(Ic.Check, null, tint = l.accent, modifier = Modifier.size(16.dp))
    }
}

/** A removable chip for a filter that is set. */
@Composable
fun FilterChip(label: String, onRemove: () -> Unit) {
    val l = LocalLain.current
    Row(
        Modifier.clip(CircleShape).background(l.accent.copy(alpha = 0.1f)).border(1.dp, l.accent.copy(alpha = 0.4f), CircleShape)
            .clickable(onClick = onRemove).padding(start = 10.dp, end = 6.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(label, color = l.text, fontSize = 12.sp)
        Icon(Ic.Close, null, tint = l.text.copy(alpha = 0.7f), modifier = Modifier.size(12.dp))
    }
}

@Composable
fun SheetHeading(text: String) {
    Mono(text, Modifier.padding(start = 8.dp, top = 18.dp, bottom = 6.dp))
}

@Composable
fun Gap(h: Dp) = Spacer(Modifier.height(h))

@Composable
fun SectionTitle(text: String, modifier: Modifier = Modifier) {
    val l = LocalLain.current
    Text(text, modifier, color = l.text, style = MaterialTheme.typography.titleLarge.copy(fontSize = 20.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.4).sp))
}


/**
 * A PIN as it is typed: a row of dots, one filled per digit, over a field
 * that only takes the keystrokes. Four places show before anything is
 * typed; a longer PIN grows the row.
 */
@Composable
fun PinField(value: String, onChange: (String) -> Unit, onDone: () -> Unit, modifier: Modifier = Modifier, label: String = "PIN") {
    val l = LocalLain.current
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    androidx.compose.runtime.LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    Box(modifier.height(64.dp).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { runCatching { focus.requestFocus() } }, contentAlignment = Alignment.Center) {
        BasicTextField(
            value = value,
            onValueChange = { v -> onChange(v.filter { it.isDigit() }.take(12)) },
            singleLine = true,
            textStyle = TextStyle(color = Color.Transparent, fontSize = 1.sp),
            cursorBrush = SolidColor(Color.Transparent),
            // Masked for whatever reads the screen too: the digits are never in the tree.
            visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.NumberPassword, imeAction = ImeAction.Done),
            keyboardActions = androidx.compose.foundation.text.KeyboardActions(onDone = { onDone() }),
            modifier = Modifier.size(1.dp).focusRequester(focus).semantics { contentDescription = label; password() },
        )
        Row(horizontalArrangement = Arrangement.spacedBy(14.dp), verticalAlignment = Alignment.CenterVertically) {
            repeat(maxOf(4, value.length)) { i ->
                val filled = i < value.length
                Box(Modifier.size(16.dp).clip(CircleShape).background(if (filled) l.accent else Color.White.copy(alpha = 0.08f)).border(1.5.dp, if (filled) l.accent else Color.White.copy(alpha = 0.45f), CircleShape))
            }
        }
    }
}

/** A masked field for a password; it takes the focus when shown. */
@Composable
fun SecretField(value: String, onChange: (String) -> Unit, numeric: Boolean, onDone: () -> Unit, modifier: Modifier = Modifier) {
    val l = LocalLain.current
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    androidx.compose.runtime.LaunchedEffect(numeric) { runCatching { focus.requestFocus() } }
    Box(
        modifier.height(48.dp).clip(l.buttonShape).background(l.field).border(1.dp, l.line, l.buttonShape).padding(horizontal = 14.dp),
        contentAlignment = Alignment.CenterStart,
    ) {
        BasicTextField(
            value = value,
            onValueChange = onChange,
            singleLine = true,
            textStyle = TextStyle(color = l.text, fontSize = 18.sp),
            cursorBrush = SolidColor(l.accent),
            visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = androidx.compose.ui.text.input.KeyboardType.Password, imeAction = ImeAction.Done),
            keyboardActions = androidx.compose.foundation.text.KeyboardActions(onDone = { onDone() }),
            modifier = Modifier.fillMaxWidth().focusRequester(focus),
        )
    }
}

/** A labelled text field, as the web client's `Input`: the label above a quiet filled box. */
@Composable
fun Field(
    value: String,
    onChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    placeholder: String = "",
    password: Boolean = false,
    keyboardType: androidx.compose.ui.text.input.KeyboardType = androidx.compose.ui.text.input.KeyboardType.Text,
    imeAction: ImeAction = ImeAction.Done,
    onIme: () -> Unit = {},
    focus: androidx.compose.ui.focus.FocusRequester? = null,
) {
    val l = LocalLain.current
    var focused by remember { androidx.compose.runtime.mutableStateOf(false) }
    Column(modifier) {
        if (label.isNotEmpty()) Text(label, Modifier.padding(bottom = 6.dp), color = l.muted, fontSize = 14.sp, fontWeight = FontWeight.Medium)
        Box(
            Modifier.fillMaxWidth().height(48.dp).clip(l.buttonShape).background(l.field)
                .border(1.dp, if (focused) l.accent.copy(alpha = 0.7f) else l.line, l.buttonShape).padding(horizontal = 14.dp),
            contentAlignment = Alignment.CenterStart,
        ) {
            if (value.isEmpty() && placeholder.isNotEmpty()) Text(placeholder, color = l.muted.copy(alpha = 0.6f), fontSize = 16.sp, maxLines = 1)
            BasicTextField(
                value = value,
                onValueChange = onChange,
                singleLine = true,
                textStyle = TextStyle(color = l.text, fontSize = 16.sp),
                cursorBrush = SolidColor(l.accent),
                visualTransformation = if (password) androidx.compose.ui.text.input.PasswordVisualTransformation() else androidx.compose.ui.text.input.VisualTransformation.None,
                keyboardOptions = KeyboardOptions(keyboardType = if (password) androidx.compose.ui.text.input.KeyboardType.Password else keyboardType, imeAction = imeAction, autoCorrectEnabled = false),
                keyboardActions = androidx.compose.foundation.text.KeyboardActions(onAny = { onIme() }),
                modifier = Modifier.fillMaxWidth()
                    .then(if (focus != null) Modifier.focusRequester(focus) else Modifier)
                    .onFocusChanged { focused = it.isFocused }
                    .semantics { contentDescription = label.ifEmpty { placeholder } },
            )
        }
    }
}

/** Nothing here yet: a quiet dashed card with what the place is for and the way forward. */
@Composable
fun EmptyState(icon: ImageVector, title: String, body: String, modifier: Modifier = Modifier, action: (@Composable () -> Unit)? = null) {
    val l = LocalLain.current
    Column(
        modifier.fillMaxWidth().clip(l.panelShape).background(l.surface.copy(alpha = 0.55f)).border(1.dp, l.line.copy(alpha = 0.5f), l.panelShape).padding(horizontal = 28.dp, vertical = 40.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(56.dp).clip(CircleShape).background(l.raised), contentAlignment = Alignment.Center) {
            Icon(icon, null, tint = l.text, modifier = Modifier.size(22.dp))
        }
        Text(title, Modifier.padding(top = 18.dp), color = l.text, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        Text(body, Modifier.padding(top = 6.dp), color = l.muted, fontSize = 14.sp, lineHeight = 20.sp, textAlign = androidx.compose.ui.text.style.TextAlign.Center)
        if (action != null) Box(Modifier.padding(top = 18.dp)) { action() }
    }
}

/** A dashed rounded outline (the web's `border-dashed`). */
fun Modifier.dashedBorder(color: Color, radius: Dp): Modifier = drawBehind {
    val stroke = androidx.compose.ui.graphics.drawscope.Stroke(
        width = 1.dp.toPx(),
        pathEffect = androidx.compose.ui.graphics.PathEffect.dashPathEffect(floatArrayOf(6.dp.toPx(), 5.dp.toPx())),
    )
    drawRoundRect(color, style = stroke, cornerRadius = androidx.compose.ui.geometry.CornerRadius(radius.toPx()))
}

/** The web client's switch: a pill with a knob, the accent when on. */
@Composable
fun Toggle(checked: Boolean, onChange: (Boolean) -> Unit, label: String) {
    val l = LocalLain.current
    val x by androidx.compose.animation.core.animateDpAsState(if (checked) 22.dp else 2.dp, label = "toggle")
    Box(
        Modifier.width(48.dp).height(28.dp).clip(CircleShape).background(if (checked) l.accent else l.raised).border(1.dp, if (checked) Color.Transparent else l.line, CircleShape)
            .clickable(onClickLabel = label) { onChange(!checked) }
            .semantics { contentDescription = label; stateDescription = if (checked) "on" else "off" },
        contentAlignment = Alignment.CenterStart,
    ) { Box(Modifier.padding(start = x).size(24.dp).clip(CircleShape).background(if (checked) l.onAccent else l.muted)) }
}

/** A group of settings: a mono heading over rows separated by hairlines. */
@Composable
fun SettingsGroup(title: String, content: @Composable ColumnScope.() -> Unit) {
    val l = LocalLain.current
    Column(Modifier.fillMaxWidth().padding(top = 26.dp)) {
        Mono(title, Modifier.padding(bottom = 10.dp), color = l.text, size = 10)
        Box(Modifier.fillMaxWidth().height(1.dp).background(l.line))
        content()
    }
}

/** One setting: its name and what it does on the left, the control on the right or underneath. */
@Composable
fun SettingRow(label: String, hint: String = "", below: Boolean = false, control: @Composable () -> Unit) {
    val l = LocalLain.current
    Column {
        if (below) {
            Column(Modifier.fillMaxWidth().padding(vertical = 14.dp)) {
                Text(label, color = l.text, fontSize = 15.sp, fontWeight = FontWeight.Medium)
                if (hint.isNotEmpty()) Text(hint, Modifier.padding(top = 3.dp), color = l.muted, fontSize = 13.sp, lineHeight = 18.sp)
                Box(Modifier.padding(top = 10.dp)) { control() }
            }
        } else {
            Row(Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f).padding(end = 14.dp)) {
                    Text(label, color = l.text, fontSize = 15.sp, fontWeight = FontWeight.Medium)
                    if (hint.isNotEmpty()) Text(hint, Modifier.padding(top = 3.dp), color = l.muted, fontSize = 13.sp, lineHeight = 18.sp)
                }
                control()
            }
        }
        Box(Modifier.fillMaxWidth().height(1.dp).background(l.line.copy(alpha = 0.6f)))
    }
}

/** A few choices in a row, one of them on (the web's segmented control). */
@Composable
fun <T> Segmented(options: List<Pair<T, String>>, selected: T, onPick: (T) -> Unit, modifier: Modifier = Modifier) {
    val l = LocalLain.current
    Row(modifier.clip(l.buttonShape).background(l.field).padding(2.dp), horizontalArrangement = Arrangement.spacedBy(2.dp)) {
        for ((value, label) in options) {
            val on = value == selected
            Box(
                Modifier.heightIn(min = 40.dp).clip(RoundedCornerShape(5.dp)).background(if (on) l.accent.copy(alpha = 0.2f) else Color.Transparent)
                    .border(1.dp, if (on) l.accent else Color.Transparent, RoundedCornerShape(5.dp))
                    .clickable { onPick(value) }.padding(horizontal = 14.dp)
                    .semantics { stateDescription = if (on) "selected" else "not selected" },
                contentAlignment = Alignment.Center,
            ) { Text(label, color = if (on) l.text else l.muted, fontSize = 13.sp, fontWeight = if (on) FontWeight.SemiBold else FontWeight.Medium, maxLines = 1) }
        }
    }
}

/** A placeholder that breathes while the real thing loads. */
@Composable
fun Skeleton(modifier: Modifier = Modifier) {
    val l = LocalLain.current
    val t = androidx.compose.animation.core.rememberInfiniteTransition(label = "skeleton")
    val a by t.animateFloat(
        0.35f, 0.75f,
        androidx.compose.animation.core.infiniteRepeatable(androidx.compose.animation.core.tween(900), androidx.compose.animation.core.RepeatMode.Reverse),
        label = "skeleton",
    )
    Box(modifier.clip(l.cardShape).background(l.raised.copy(alpha = a)))
}

/** A thin slider: the web player's seek bar, for anything with a position. */
@Composable
fun ThinSlider(fraction: Float, onChange: (Float) -> Unit, onCommit: (Float) -> Unit, modifier: Modifier = Modifier, reversed: Boolean = false, label: String = "") {
    val l = LocalLain.current
    var drag by remember { androidx.compose.runtime.mutableStateOf<Float?>(null) }
    val shown = (drag ?: fraction).coerceIn(0f, 1f)
    androidx.compose.foundation.layout.BoxWithConstraints(
        modifier.height(36.dp).semantics { contentDescription = label }
            .pointerInput(reversed) {
                detectTapGestures { o -> val f = (o.x / size.width).coerceIn(0f, 1f).let { if (reversed) 1f - it else it }; onChange(f); onCommit(f) }
            }
            .pointerInput(reversed) {
                detectHorizontalDragGestures(
                    onDragEnd = { drag?.let(onCommit); drag = null },
                    onDragCancel = { drag = null },
                ) { change, _ -> val f = (change.position.x / size.width).coerceIn(0f, 1f).let { if (reversed) 1f - it else it }; drag = f; onChange(f) }
            },
        contentAlignment = Alignment.CenterStart,
    ) {
        val w = maxWidth
        val x = if (reversed) 1f - shown else shown
        Box(Modifier.fillMaxWidth().height(3.dp).clip(CircleShape).background(Color.White.copy(alpha = 0.22f)))
        if (reversed) Box(Modifier.align(Alignment.CenterEnd).fillMaxWidth(shown).height(3.dp).clip(CircleShape).background(l.accent))
        else Box(Modifier.fillMaxWidth(shown).height(3.dp).clip(CircleShape).background(l.accent))
        Box(Modifier.offset(x = (w - 14.dp) * x).size(if (drag != null) 18.dp else 14.dp).clip(CircleShape).background(l.accent))
    }
}
