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

/** "Account toevoegen" for an IMAP/SMTP account, such as MXroute. Microsoft 365 and Gmail follow. */
@Composable
fun AddAccountScreen(store: Store) {
    val h = LocalHelder.current
    var name by rememberSaveable { mutableStateOf("") }
    var email by rememberSaveable { mutableStateOf("") }
    // The password is never put in saved instance state.
    var password by androidx.compose.runtime.remember { mutableStateOf("") }
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
                IconButton(onClick = { store.screen = MailScreen.Folders }) { Icon(Icons.Outlined.Close, stringResource(R.string.cancel)) }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        Column(
            Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
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
