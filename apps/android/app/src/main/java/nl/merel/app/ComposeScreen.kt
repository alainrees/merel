@file:OptIn(ExperimentalMaterial3Api::class)

package nl.merel.app

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Send
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp

/** The address list `field` with its last, half-typed address replaced by `address`. */
private fun withPicked(field: String, address: String): String =
    (field.split(',', ';').map { it.trim() }.dropLast(1).filter { it.isNotEmpty() } + address).joinToString(", ") + ", "

/** Nieuw bericht, Beantwoorden, Allen beantwoorden or Doorsturen: Van, Aan, CC/BCC, Onderwerp and the text. */
@Composable
fun ComposeScreen(store: Store, original: ComposeForm) {
    val h = LocalHelder.current
    var account by rememberSaveable { mutableStateOf(original.accountId.ifEmpty { store.sendPlaces.firstOrNull()?.accountId ?: "" }) }
    var to by rememberSaveable { mutableStateOf(original.to) }
    var cc by rememberSaveable { mutableStateOf(original.cc) }
    var bcc by rememberSaveable { mutableStateOf(original.bcc) }
    var subject by rememberSaveable { mutableStateOf(original.subject) }
    var text by rememberSaveable { mutableStateOf(original.body) }
    var copies by rememberSaveable { mutableStateOf(original.cc.isNotEmpty() || original.bcc.isNotEmpty()) }
    // The address field being typed in, for the suggestions under it.
    var typing by rememberSaveable { mutableStateOf("to") }
    var problem by rememberSaveable { mutableStateOf<String?>(null) }
    var confirmDiscard by rememberSaveable { mutableStateOf(false) }
    val chooseAccount = stringResource(R.string.choose_send_account)
    val changed = to != original.to || cc != original.cc || bcc != original.bcc || subject != original.subject || text != original.body
    val close = { if (changed) confirmDiscard = true else store.composing = null }
    BackHandler(onBack = close)

    val title = when {
        original.inReplyTo != null -> R.string.reply
        original.subject.isNotEmpty() -> R.string.forward
        else -> R.string.new_message
    }
    Column(Modifier.fillMaxSize().imePadding()) {
        TopAppBar(
            title = { Text(stringResource(title)) },
            navigationIcon = { IconButton(onClick = close) { Icon(Icons.Outlined.Close, stringResource(R.string.cancel)) } },
            actions = {
                IconButton(
                    onClick = {
                        store.sendMail(ComposeForm(account, to, cc, bcc, subject, text, original.inReplyTo))
                            .onSuccess { store.composing = null }
                            .onFailure { e -> problem = if (e is CoreException.Invalid) chooseAccount else e.message }
                    },
                    enabled = to.isNotBlank(),
                ) { Icon(Icons.AutoMirrored.Outlined.Send, stringResource(R.string.send), tint = if (to.isNotBlank()) h.ink else h.dim) }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        Column(Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (store.sendPlaces.size > 1) {
                Choice(stringResource(R.string.from), store.sendPlaces.map { it.accountId to it.address }, account) { account = it }
            }
            val email = KeyboardOptions(keyboardType = KeyboardType.Email, autoCorrectEnabled = false)
            @Composable
            fun Suggestions(field: String, value: String, pick: (String) -> Unit) {
                if (typing != field) return
                val typed = value.split(',', ';').last().trim()
                if (typed.length < 2) return
                store.contacts.filter { c -> c.name.contains(typed, true) || c.emails.any { it.contains(typed, true) } }.take(4).forEach { c ->
                    val address = c.emails.firstOrNull() ?: return@forEach
                    Column(Modifier.fillMaxWidth().clickable { pick(withPicked(value, address)) }.padding(vertical = 6.dp)) {
                        Text(c.name)
                        Text(address, color = h.dim, style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
            OutlinedTextField(
                to, { to = it; typing = "to" }, label = { Text(stringResource(R.string.to)) }, keyboardOptions = email, modifier = Modifier.fillMaxWidth(),
            )
            Suggestions("to", to) { to = it }
            if (copies) {
                OutlinedTextField(
                    cc, { cc = it; typing = "cc" }, label = { Text(stringResource(R.string.cc)) }, keyboardOptions = email, modifier = Modifier.fillMaxWidth(),
                )
                Suggestions("cc", cc) { cc = it }
                OutlinedTextField(
                    bcc, { bcc = it; typing = "bcc" }, label = { Text(stringResource(R.string.bcc)) }, keyboardOptions = email, modifier = Modifier.fillMaxWidth(),
                )
                Suggestions("bcc", bcc) { bcc = it }
            } else {
                TextButton(onClick = { copies = true }) { Text(stringResource(R.string.add_cc_bcc), color = h.ink) }
            }
            OutlinedTextField(subject, { subject = it }, label = { Text(stringResource(R.string.subject)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(text, { text = it }, label = { Text(stringResource(R.string.message)) }, minLines = 10, modifier = Modifier.fillMaxWidth())
            problem?.let { Text(it, color = h.flag) }
        }
    }
    if (confirmDiscard) {
        AlertDialog(
            onDismissRequest = { confirmDiscard = false },
            title = { Text(stringResource(R.string.discard_message)) },
            confirmButton = {
                TextButton(onClick = { confirmDiscard = false; store.composing = null }) { Text(stringResource(R.string.discard), color = h.flag) }
            },
            dismissButton = { TextButton(onClick = { confirmDiscard = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
