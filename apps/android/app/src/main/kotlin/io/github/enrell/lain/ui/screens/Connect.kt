package io.github.enrell.lain.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import io.github.enrell.lain.R
import io.github.enrell.lain.ui.AppState
import io.github.enrell.lain.ui.AppViewModel
import io.github.enrell.lain.ui.theme.LocalLain

/** Server URL (manual discovery, D-179). Enter connects. */
@Composable
fun ConnectScreen(state: AppState, vm: AppViewModel) {
    var url by rememberSaveable { mutableStateOf(state.server.ifEmpty { "http://" }) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Shell(stringResource(R.string.connect_title), stringResource(R.string.connect_hint)) {
        OutlinedTextField(
            value = url,
            onValueChange = { url = it },
            label = { Text(stringResource(R.string.connect_url)) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Go),
            keyboardActions = KeyboardActions(onGo = { vm.connect(url) }),
            modifier = Modifier.fillMaxWidth().focusRequester(focus),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(LocalLain.current.sp(2)), verticalAlignment = Alignment.CenterVertically) {
            Action(stringResource(R.string.connect_go), "Enter", { vm.connect(url) }, primary = true, enabled = !state.busy)
            if (state.busy) Label(stringResource(R.string.working))
        }
        ErrorLine(state.error)
    }
}

/** Username and password; Enter moves on, then logs in. */
@Composable
fun LoginScreen(state: AppState, vm: AppViewModel) {
    var user by rememberSaveable { mutableStateOf("") }
    var pass by remember { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    val passFocus = remember { FocusRequester() }
    LaunchedEffect(Unit) { focus.requestFocus() }
    Shell(stringResource(R.string.login_title), state.server) {
        if (state.setupRequired) {
            Text(stringResource(R.string.login_setup_required), color = LocalLain.current.warn, style = MaterialTheme.typography.bodyMedium)
        }
        OutlinedTextField(
            value = user,
            onValueChange = { user = it },
            label = { Text(stringResource(R.string.login_user)) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next),
            keyboardActions = KeyboardActions(onNext = { passFocus.requestFocus() }),
            modifier = Modifier.fillMaxWidth().focusRequester(focus),
        )
        OutlinedTextField(
            value = pass,
            onValueChange = { pass = it },
            label = { Text(stringResource(R.string.login_password)) },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { vm.login(user, pass) }),
            modifier = Modifier.fillMaxWidth().focusRequester(passFocus),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(LocalLain.current.sp(2)), verticalAlignment = Alignment.CenterVertically) {
            Action(stringResource(R.string.login_go), "Enter", { vm.login(user, pass) }, primary = true, enabled = !state.busy)
            Action(stringResource(R.string.login_other_server), null, { vm.changeServer() })
        }
        ErrorLine(state.error)
    }
}

@Composable
private fun Shell(title: String, hint: String, content: @Composable () -> Unit) {
    val l = LocalLain.current
    Column(
        Modifier.fillMaxSize().imePadding().padding(l.sp(5)),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.Start,
    ) {
        Column(Modifier.widthIn(max = 520.dp), verticalArrangement = Arrangement.spacedBy(l.sp(3))) {
            Label("lain")
            Text(title, style = MaterialTheme.typography.displayMedium)
            if (hint.isNotEmpty()) Text(hint, color = l.muted, style = MaterialTheme.typography.bodyMedium)
            content()
        }
    }
}
