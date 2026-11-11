package io.github.enrell.lain.ui.screens

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
        Btn.Secondary -> Triple(l.raised, l.text, l.line)
        Btn.Ghost -> Triple(Color.Transparent, l.muted, Color.Transparent)
        Btn.Danger -> Triple(l.danger.copy(alpha = 0.15f), l.danger, l.danger.copy(alpha = 0.3f))
    }
    Row(
        modifier
            .height(if (large) 48.dp else 44.dp)
            .clip(shape)
            .background(bg.copy(alpha = bg.alpha * if (enabled) 1f else 0.5f))
            .border(1.dp, edge, shape)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = if (large) 24.dp else 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
    ) {
        if (icon != null) Icon(icon, null, tint = fg, modifier = Modifier.size(16.dp))
        Text(text, color = fg, fontSize = if (large) 16.sp else 14.sp, fontWeight = FontWeight.Medium, maxLines = 1)
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
        style = TextStyle(fontFamily = l.mono, fontSize = size.sp, letterSpacing = (size * 0.18f).sp, color = color ?: l.muted, fontWeight = FontWeight.Medium),
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
            Art(artId, Modifier.fillMaxSize())
            if (badge != null) {
                Text(
                    badge,
                    Modifier.padding(8.dp).clip(CircleShape).background(Color.Black.copy(alpha = 0.65f)).padding(horizontal = 8.dp, vertical = 2.dp),
                    style = TextStyle(fontFamily = l.mono, fontSize = 10.sp, fontWeight = FontWeight.SemiBold, color = Color.White),
                )
            }
            if (progress > 0f) {
                Box(Modifier.align(Alignment.BottomStart).fillMaxWidth().height(4.dp).background(Color.Black.copy(alpha = 0.5f))) {
                    Box(Modifier.fillMaxWidth(progress.coerceIn(0f, 1f)).height(4.dp).background(l.accent))
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
fun Sheet(title: String, onDismiss: () -> Unit, footer: (@Composable () -> Unit)? = null, content: @Composable ColumnScope.() -> Unit) {
    val l = LocalLain.current
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = l.surface,
        contentColor = l.text,
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
            Column(
                Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(bottom = 12.dp),
                content = content,
            )
            if (footer != null) Box(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 12.dp)) { footer() }
        }
    }
}

/** The web's search field: a quiet filled box with the glass on the left. */
@Composable
fun SearchField(value: String, onChange: (String) -> Unit, placeholder: String, modifier: Modifier = Modifier) {
    val l = LocalLain.current
    Row(
        modifier.height(44.dp).clip(l.buttonShape).background(l.field).border(1.dp, l.line, l.buttonShape).padding(start = 12.dp),
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
                modifier = Modifier.fillMaxWidth(),
            )
        }
        if (value.isNotEmpty()) IconBtn(Ic.Close, "Clear", { onChange("") })
    }
}

/** A filter option row: label, the count it would leave, pressed state. */
@Composable
fun OptionRow(label: String, count: Int?, on: Boolean, onClick: () -> Unit, enabled: Boolean = true) {
    val l = LocalLain.current
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 44.dp)
            .clip(l.buttonShape)
            .background(if (on) l.accent.copy(alpha = 0.15f) else Color.Transparent)
            .clickable(enabled = enabled || on, onClick = onClick)
            .padding(horizontal = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val c = if (on) l.text else l.muted.copy(alpha = if (enabled) 1f else 0.35f)
        Text(label, Modifier.weight(1f), color = c, fontSize = 14.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (count != null) Text(count.toString(), style = TextStyle(fontFamily = l.mono, fontSize = 11.sp, color = c.copy(alpha = c.alpha * 0.7f)))
    }
}

/** A choice among a few, as outlined tiles (the sheet's Sort and Tiles). */
@Composable
fun ChoiceTile(label: String, on: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val l = LocalLain.current
    Box(
        modifier
            .heightIn(min = 44.dp)
            .clip(l.buttonShape)
            .background(if (on) l.accent.copy(alpha = 0.15f) else Color.Transparent)
            .border(1.dp, if (on) l.accent.copy(alpha = 0.6f) else l.line, l.buttonShape)
            .clickable(onClick = onClick)
            .padding(horizontal = 12.dp),
        contentAlignment = Alignment.CenterStart,
    ) { Text(label, color = if (on) l.text else l.muted, fontSize = 14.sp, maxLines = 1) }
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


/** A masked field for a PIN or a password; it takes the focus when shown. */
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
            onValueChange = { v -> onChange(if (numeric) v.filter { it.isDigit() }.take(12) else v) },
            singleLine = true,
            textStyle = TextStyle(color = l.text, fontSize = 18.sp, letterSpacing = if (numeric) 6.sp else 0.sp),
            cursorBrush = SolidColor(l.accent),
            visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(
                keyboardType = if (numeric) androidx.compose.ui.text.input.KeyboardType.NumberPassword else androidx.compose.ui.text.input.KeyboardType.Password,
                imeAction = ImeAction.Done,
            ),
            keyboardActions = androidx.compose.foundation.text.KeyboardActions(onDone = { onDone() }),
            modifier = Modifier.fillMaxWidth().focusRequester(focus),
        )
    }
}
