@file:OptIn(ExperimentalMaterial3Api::class)

package nl.merel.app

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import uniffi.merel_mobile.ConnectionSecurity
import uniffi.merel_mobile.CoreException
import uniffi.merel_mobile.ImapAccount
import uniffi.merel_mobile.ServerSettings
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.TextButton
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import uniffi.merel_mobile.SignInProvider
import uniffi.merel_mobile.SignInState

/**
 * "Account toevoegen": Microsoft 365 and Gmail sign in in the browser, any other account (such
 * as MXroute) with IMAP/SMTP and a password.
 */
@Composable
fun AddAccountScreen(store: Store) {
    val h = LocalHelder.current
    // 0 Microsoft 365, 1 Gmail, 2 another account.
    var kind by rememberSaveable { mutableIntStateOf(0) }
    var name by rememberSaveable { mutableStateOf("") }
    var email by rememberSaveable { mutableStateOf("") }
    // The password is never put in saved instance state.
    var password by remember { mutableStateOf("") }
    var username by rememberSaveable { mutableStateOf("") }
    var imapHost by rememberSaveable { mutableStateOf("") }
    var imapPort by rememberSaveable { mutableStateOf("993") }
    var imapTls by rememberSaveable { mutableStateOf(true) }
    var smtpHost by rememberSaveable { mutableStateOf("") }
    var smtpPort by rememberSaveable { mutableStateOf("465") }
    var smtpTls by rememberSaveable { mutableStateOf(true) }
    val fieldNames = mapOf(
        "email" to stringResource(R.string.field_email), "username" to stringResource(R.string.field_username),
        "password" to stringResource(R.string.field_password), "incoming" to stringResource(R.string.field_incoming),
        "outgoing" to stringResource(R.string.field_outgoing),
    )
    val checkField = stringResource(R.string.check_field, "%s")

    Column(Modifier.fillMaxSize().imePadding()) {
        TopAppBar(
            title = { Text(stringResource(R.string.add_account)) },
            navigationIcon = {
                IconButton(onClick = {
                    store.clearSignIn()
                    store.screen = MailScreen.Folders
                }) { Icon(Icons.Outlined.Close, stringResource(R.string.cancel)) }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        Column(
            Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                listOf(R.string.microsoft_365, R.string.gmail, R.string.other_account).forEachIndexed { i, label ->
                    SegmentedButton(kind == i, {
                        kind = i
                        store.clearSignIn()
                    }, SegmentedButtonDefaults.itemShape(i, 3)) { Text(stringResource(label)) }
                }
            }
            if (kind != 2) {
                SignInFields(store, gmail = kind == 1)
                return@Column
            }
            Field(name, { name = it }, R.string.name)
            Field(email, {
                email = it
                // mail.<domain> for both servers, the most common setup, until the person types their own.
                val domain = it.substringAfter('@', "")
                if (domain.contains('.')) {
                    if (imapHost.isEmpty() || imapHost.startsWith("mail.")) imapHost = "mail.$domain"
                    if (smtpHost.isEmpty() || smtpHost.startsWith("mail.")) smtpHost = "mail.$domain"
                }
            }, R.string.email, KeyboardType.Email)
            OutlinedTextField(
                password, { password = it }, label = { Text(stringResource(R.string.password)) }, singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth(),
            )
            Heading(R.string.incoming)
            Field(imapHost, { imapHost = it }, R.string.server, KeyboardType.Uri)
            Field(imapPort, { imapPort = it.filter(Char::isDigit) }, R.string.port, KeyboardType.Number)
            SecurityChoice(imapTls) { imapTls = it }
            Heading(R.string.outgoing)
            Field(smtpHost, { smtpHost = it }, R.string.server, KeyboardType.Uri)
            Field(smtpPort, { smtpPort = it.filter(Char::isDigit) }, R.string.port, KeyboardType.Number)
            SecurityChoice(smtpTls) { smtpTls = it }
            Field(username, { username = it }, R.string.username)
            Text(stringResource(R.string.password_note), color = h.dim, style = MaterialTheme.typography.bodySmall)
            Button(
                onClick = {
                    val form = ImapAccount(
                        name, email, username,
                        ServerSettings(imapHost, imapPort.toUShortOrNull() ?: 0u, if (imapTls) ConnectionSecurity.TLS else ConnectionSecurity.START_TLS),
                        ServerSettings(smtpHost, smtpPort.toUShortOrNull() ?: 0u, if (smtpTls) ConnectionSecurity.TLS else ConnectionSecurity.START_TLS),
                    )
                    store.addAccount(form, password)
                        .onSuccess { password = "" }
                        .onFailure { e ->
                            store.problem = if (e is CoreException.Invalid) checkField.format(fieldNames[e.field] ?: e.field) else e.message
                        }
                },
                enabled = email.isNotBlank() && password.isNotEmpty() && imapHost.isNotBlank() && smtpHost.isNotBlank(),
                colors = ButtonDefaults.buttonColors(containerColor = h.accent, contentColor = h.onAccent),
                modifier = Modifier.fillMaxWidth(),
            ) { Text(stringResource(R.string.add)) }
        }
    }
}

/** Name and address, for Gmail the person's own client ID and secret, and the sign-in button. */
@Composable
private fun SignInFields(store: Store, gmail: Boolean) {
    val h = LocalHelder.current
    var name by rememberSaveable { mutableStateOf("") }
    var email by rememberSaveable { mutableStateOf("") }
    var clientId by rememberSaveable { mutableStateOf("") }
    var clientSecret by remember { mutableStateOf("") }
    val provider = if (gmail) "Google" else "Microsoft"
    val fieldNames = mapOf(
        "email" to stringResource(R.string.field_email), "client_id" to stringResource(R.string.field_client_id),
        "client_secret" to stringResource(R.string.field_client_secret),
    )
    val checkField = stringResource(R.string.check_field, "%s")
    LaunchedEffect(store.signIn) {
        if (store.signIn is SignInState.Done) {
            store.clearSignIn()
            store.screen = MailScreen.Folders
        }
    }
    Field(name, { name = it }, R.string.name)
    Field(email, { email = it }, R.string.email, KeyboardType.Email)
    if (gmail) {
        Field(clientId, { clientId = it }, R.string.client_id)
        OutlinedTextField(
            clientSecret, { clientSecret = it }, label = { Text(stringResource(R.string.client_secret)) }, singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password), modifier = Modifier.fillMaxWidth(),
        )
        Text(stringResource(R.string.google_help), color = h.dim, style = MaterialTheme.typography.bodySmall)
    } else {
        Text(stringResource(R.string.microsoft_help), color = h.dim, style = MaterialTheme.typography.bodySmall)
    }
    if (store.signIn is SignInState.Waiting) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            CircularProgressIndicator(Modifier.size(24.dp))
            Text(stringResource(R.string.waiting_for_sign_in, provider), modifier = Modifier.padding(start = 12.dp).weight(1f))
            TextButton(onClick = { store.clearSignIn() }) { Text(stringResource(R.string.stop)) }
        }
    } else {
        Button(
            onClick = {
                store.signIn(if (gmail) SignInProvider.GMAIL else SignInProvider.MICROSOFT365, name, email, clientId, clientSecret)
                    .onFailure { e -> store.problem = if (e is CoreException.Invalid) checkField.format(fieldNames[e.field] ?: e.field) else e.message }
            },
            enabled = email.isNotBlank(),
            colors = ButtonDefaults.buttonColors(containerColor = h.accent, contentColor = h.onAccent),
            modifier = Modifier.fillMaxWidth(),
        ) { Text(stringResource(R.string.sign_in_with, provider)) }
    }
    (store.signIn as? SignInState.Failed)?.let { Text(it.reason, color = h.flag) }
}

@Composable
private fun Field(value: String, onChange: (String) -> Unit, label: Int, type: KeyboardType = KeyboardType.Text) {
    OutlinedTextField(
        value, onChange, label = { Text(stringResource(label)) }, singleLine = true,
        keyboardOptions = KeyboardOptions(keyboardType = type, autoCorrectEnabled = false), modifier = Modifier.fillMaxWidth(),
    )
}

@Composable
private fun Heading(text: Int) {
    Text(stringResource(text), style = MaterialTheme.typography.titleSmall, modifier = Modifier.padding(top = 8.dp).semantics { heading() })
}

@Composable
private fun SecurityChoice(tls: Boolean, onChange: (Boolean) -> Unit) {
    Row {
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            SegmentedButton(tls, { onChange(true) }, SegmentedButtonDefaults.itemShape(0, 2)) { Text("TLS") }
            SegmentedButton(!tls, { onChange(false) }, SegmentedButtonDefaults.itemShape(1, 2)) { Text("STARTTLS") }
        }
    }
}
