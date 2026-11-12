package io.github.enrell.lain.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.enrell.lain.R
import io.github.enrell.lain.ui.AppState
import io.github.enrell.lain.ui.AppViewModel
import io.github.enrell.lain.ui.theme.LocalLain

/**
 * Server URL (manual discovery, D-179). It starts empty, not at a scheme:
 * an address typed without one is `http` on the LAN and `https` anywhere
 * else, the core's rule (D-277), and a prefilled scheme would overrule it.
 */
@Composable
fun ConnectScreen(state: AppState, vm: AppViewModel) {
    var url by rememberSaveable { mutableStateOf(state.server) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    Door(stringResource(R.string.connect_title), stringResource(R.string.connect_hint)) {
        Field(
            url, { url = it }, stringResource(R.string.connect_url), Modifier.fillMaxWidth(),
            placeholder = "192.168.1.10:9360", keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go, onIme = { vm.connect(url) }, focus = focus,
        )
        ErrorLine(state.error)
        WButton(stringResource(if (state.busy) R.string.working else R.string.connect_go), { vm.connect(url) }, Modifier.fillMaxWidth(), large = true, enabled = !state.busy)
    }
}

/** Username and password; the keyboard's action moves on, then signs in. */
@Composable
fun LoginScreen(state: AppState, vm: AppViewModel) {
    val l = LocalLain.current
    var user by rememberSaveable { mutableStateOf("") }
    var pass by remember { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    val passFocus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    Door(stringResource(R.string.login_title), state.server) {
        if (state.setupRequired) Text(stringResource(R.string.login_setup_required), color = l.warn, fontSize = 14.sp, lineHeight = 20.sp)
        Field(user, { user = it }, stringResource(R.string.login_user), Modifier.fillMaxWidth(), imeAction = ImeAction.Next, onIme = { runCatching { passFocus.requestFocus() } }, focus = focus)
        Field(pass, { pass = it }, stringResource(R.string.login_password), Modifier.fillMaxWidth(), password = true, onIme = { vm.login(user, pass) }, focus = passFocus)
        ErrorLine(state.error)
        WButton(stringResource(if (state.busy) R.string.working else R.string.login_go), { vm.login(user, pass) }, Modifier.fillMaxWidth(), large = true, enabled = !state.busy && user.isNotBlank() && pass.isNotEmpty())
        Text(
            stringResource(R.string.login_other_server),
            Modifier.fillMaxWidth().clip(l.buttonShape).clickable { vm.changeServer() }.padding(vertical = 12.dp),
            color = l.muted, fontSize = 14.sp, textAlign = TextAlign.Center,
        )
    }
}

/** The way in, as the web client's: the name, a line under it, then one card. */
@Composable
private fun Door(title: String, hint: String, content: @Composable () -> Unit) {
    val l = LocalLain.current
    Column(
        Modifier.fillMaxSize().imePadding().verticalScroll(rememberScrollState()).padding(horizontal = 20.dp, vertical = 32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("lain", color = l.text, fontSize = 26.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 9.sp)
        Text(stringResource(R.string.tagline), Modifier.padding(top = 10.dp, bottom = 28.dp), color = l.muted, fontSize = 14.sp)
        Column(
            Modifier.widthIn(max = 440.dp).fillMaxWidth().clip(l.panelShape).background(l.surface).border(1.dp, l.line, l.panelShape).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Column {
                Text(title, color = l.text, fontSize = 20.sp, fontWeight = FontWeight.SemiBold, letterSpacing = (-0.3).sp)
                if (hint.isNotEmpty()) Text(hint, Modifier.padding(top = 4.dp), color = l.muted, fontSize = 13.sp, lineHeight = 18.sp)
            }
            content()
        }
    }
}
