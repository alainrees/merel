@file:OptIn(ExperimentalMaterial3Api::class)

package nl.merel.app

import android.content.Intent
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Call
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Email
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.ui.unit.dp
import uniffi.merel_mobile.ContactForm
import uniffi.merel_mobile.ContactRow
import uniffi.merel_mobile.CoreException
import uniffi.merel_mobile.PhoneKind
import uniffi.merel_mobile.PhoneNumber

/** Contacts: the list by name with search, a contact's card, and a form to add or change one. */
@Composable
fun ContactsTab(store: Store) {
    store.editingContact?.let {
        ContactEditor(store, it)
        return
    }
    store.openContact?.let { id ->
        ContactCard(store, id)
        return
    }
    val h = LocalHelder.current
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            TextField(
                value = store.contactQuery, onValueChange = { store.searchContacts(it) }, singleLine = true,
                placeholder = { Text(stringResource(R.string.search_contacts)) },
                leadingIcon = { Icon(Icons.Outlined.Search, contentDescription = null) },
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent,
                    focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent,
                ),
                modifier = Modifier.padding(16.dp, 8.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(h.card)
                    .border(1.dp, h.border, RoundedCornerShape(28.dp)),
            )
            Text(
                stringResource(R.string.tab_contacts), style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(20.dp, 4.dp).semantics { heading() },
            )
            if (store.contacts.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(stringResource(if (store.contactQuery.isBlank()) R.string.no_contacts else R.string.nothing_found), color = h.dim)
                }
            } else {
                LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
                    items(store.contacts, key = { it.id }) { c ->
                        Row(
                            Modifier.fillMaxWidth().clickable { store.openContact = c.id }.padding(vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            ContactPicture(store, c, 40)
                            Column(Modifier.padding(start = 12.dp)) {
                                Text(c.name)
                                val line = c.company.ifEmpty { c.emails.firstOrNull() ?: "" }
                                if (line.isNotEmpty()) Text(line, color = h.dim, style = MaterialTheme.typography.bodySmall)
                            }
                        }
                        HorizontalDivider(color = h.border)
                    }
                }
            }
        }
        FloatingActionButton(
            onClick = { store.editingContact = ContactForm(null, null, "", "", "", listOf(""), listOf(PhoneNumber(PhoneKind.MOBILE, "")), "") },
            containerColor = h.accent,
            modifier = Modifier.align(Alignment.BottomEnd).padding(20.dp),
        ) { Icon(Icons.Outlined.Add, stringResource(R.string.new_contact), tint = h.onAccent) }
    }
}

/** The contact's photo, or its initials on a coloured circle. */
@Composable
private fun ContactPicture(store: Store, c: ContactRow, size: Int) {
    val bitmap = remember(c.id, c.hasPhoto) {
        if (c.hasPhoto) store.contactPhoto(c.id)?.let { BitmapFactory.decodeByteArray(it, 0, it.size) } else null
    }
    if (bitmap != null) {
        Image(bitmap.asImageBitmap(), contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.size(size.dp).clip(CircleShape))
    } else {
        Avatar(c.name, c.emails.firstOrNull() ?: c.name, size)
    }
}

@Composable
private fun phoneKindText(kind: PhoneKind): String = stringResource(
    when (kind) {
        PhoneKind.MOBILE -> R.string.phone_mobile
        PhoneKind.WORK -> R.string.place_work
        PhoneKind.HOME -> R.string.place_home
        PhoneKind.OTHER -> R.string.place_other
    },
)

/** A contact's card: picture, name, job, and every way to reach them. */
@Composable
private fun ContactCard(store: Store, id: String) {
    val h = LocalHelder.current
    val context = LocalContext.current
    BackHandler { store.openContact = null }
    val c = store.contact(id)
    fun open(uri: String) = runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(uri))) }
    fun tel(number: String) = "tel:" + number.filter { it.isDigit() || it == '+' }
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = {},
            navigationIcon = { IconButton(onClick = { store.openContact = null }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back)) } },
            actions = {
                if (c != null) {
                    IconButton(onClick = {
                        store.editingContact = ContactForm(c.id, c.accountId, c.name, c.company, c.jobTitle, c.emails.ifEmpty { listOf("") }, c.phones, c.notes)
                    }) { Icon(Icons.Outlined.Edit, stringResource(R.string.edit_contact)) }
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        if (c == null) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(stringResource(R.string.contact_gone), color = h.dim) }
            return@Column
        }
        Column(Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(6.dp)) {
                ContactPicture(store, c, 88)
                Text(c.name, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
                val job = listOf(c.jobTitle, c.department, c.company).filter { it.isNotEmpty() }.joinToString(" · ")
                if (job.isNotEmpty()) Text(job, color = h.dim)
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    c.emails.firstOrNull()?.let { email ->
                        TextButton(onClick = { open("mailto:$email") }) {
                            Icon(Icons.Outlined.Email, contentDescription = null)
                            Text(" " + stringResource(R.string.mail_action))
                        }
                    }
                    c.phones.firstOrNull()?.let { phone ->
                        TextButton(onClick = { open(tel(phone.number)) }) {
                            Icon(Icons.Outlined.Call, contentDescription = null)
                            Text(" " + stringResource(R.string.call_action))
                        }
                    }
                }
            }
            if (c.emails.isNotEmpty()) {
                CardSection(stringResource(R.string.email)) {
                    c.emails.forEach { email -> Text(email, color = h.ink, modifier = Modifier.clickable { open("mailto:$email") }.padding(vertical = 6.dp)) }
                }
            }
            if (c.phones.isNotEmpty()) {
                CardSection(stringResource(R.string.phone)) {
                    c.phones.forEach { p ->
                        Row(Modifier.fillMaxWidth().clickable { open(tel(p.number)) }.padding(vertical = 6.dp)) {
                            Text(phoneKindText(p.kind), color = h.dim, modifier = Modifier.weight(1f))
                            Text(p.number, color = h.ink)
                        }
                    }
                }
            }
            if (c.addresses.isNotEmpty()) {
                CardSection(stringResource(R.string.address)) {
                    c.addresses.forEach { a ->
                        val kind = stringResource(if (a.work) R.string.place_work else if (a.home) R.string.place_home else R.string.place_other)
                        Row(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
                            Text(kind, color = h.dim, modifier = Modifier.weight(1f))
                            Text(a.lines.joinToString("\n"))
                        }
                    }
                }
            }
            c.birthday?.let { CardSection(stringResource(R.string.birthday)) { Text(it) } }
            if (c.website.isNotEmpty()) CardSection(stringResource(R.string.website)) { Text(c.website) }
            if (c.notes.isNotEmpty()) CardSection(stringResource(R.string.notes)) { Text(c.notes) }
            CardSection(stringResource(R.string.kept_in)) { Text(c.accountAddress ?: stringResource(R.string.on_this_phone)) }
        }
    }
}

@Composable
private fun CardSection(title: String, content: @Composable () -> Unit) {
    val h = LocalHelder.current
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(h.card).border(1.dp, h.border, RoundedCornerShape(12.dp)).padding(14.dp),
    ) {
        Text(title, color = h.dim, style = MaterialTheme.typography.labelLarge, modifier = Modifier.semantics { heading() })
        content()
    }
}

@Composable
private fun ContactEditor(store: Store, original: ContactForm) {
    val h = LocalHelder.current
    BackHandler { store.editingContact = null }
    var name by rememberSaveable { mutableStateOf(original.name) }
    var company by rememberSaveable { mutableStateOf(original.company) }
    var jobTitle by rememberSaveable { mutableStateOf(original.jobTitle) }
    val emails = remember { mutableStateListOf<String>().apply { addAll(original.emails) } }
    val phones = remember { mutableStateListOf<PhoneNumber>().apply { addAll(original.phones) } }
    var notes by rememberSaveable { mutableStateOf(original.notes) }
    var place by rememberSaveable { mutableStateOf(original.accountId ?: "") }
    var confirmDelete by remember { mutableStateOf(false) }
    val messages = mapOf("name" to stringResource(R.string.need_contact_name), "emails" to stringResource(R.string.bad_contact_email))
    fun save() {
        val form = ContactForm(original.id, place.ifEmpty { null }, name, company, jobTitle, emails.toList(), phones.toList(), notes)
        store.saveContact(form)
            .onSuccess { store.editingContact = null }
            .onFailure { e -> store.problem = if (e is CoreException.Invalid) messages[e.field] ?: e.message else e.message }
    }
    val kinds = listOf(PhoneKind.MOBILE, PhoneKind.WORK, PhoneKind.HOME, PhoneKind.OTHER)
    val kindLabels = kinds.map { it to phoneKindText(it) }
    Column(Modifier.fillMaxSize().imePadding()) {
        TopAppBar(
            title = { Text(stringResource(if (original.id == null) R.string.new_contact else R.string.edit_contact)) },
            navigationIcon = { IconButton(onClick = { store.editingContact = null }) { Icon(Icons.Outlined.Close, stringResource(R.string.cancel)) } },
            actions = {
                if (original.id != null) {
                    IconButton(onClick = { confirmDelete = true }) { Icon(Icons.Outlined.Delete, stringResource(R.string.delete_contact)) }
                }
                TextButton(onClick = { save() }, enabled = name.isNotBlank() || emails.any { it.isNotBlank() }) { Text(stringResource(R.string.save)) }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        Column(Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(name, { name = it }, label = { Text(stringResource(R.string.contact_name)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(company, { company = it }, label = { Text(stringResource(R.string.company)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(jobTitle, { jobTitle = it }, label = { Text(stringResource(R.string.job_title)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            emails.forEachIndexed { i, email ->
                OutlinedTextField(
                    email, { emails[i] = it }, label = { Text(stringResource(R.string.email)) }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email),
                    trailingIcon = { IconButton(onClick = { emails.removeAt(i) }) { Icon(Icons.Outlined.Close, stringResource(R.string.remove)) } },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            TextButton(onClick = { emails.add("") }) { Text(stringResource(R.string.add_email)) }
            phones.forEachIndexed { i, phone ->
                Choice(stringResource(R.string.phone), kindLabels, phone.kind) { phones[i] = phone.copy(kind = it) }
                OutlinedTextField(
                    phone.number, { phones[i] = phone.copy(number = it) }, label = { Text(stringResource(R.string.number)) }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Phone),
                    trailingIcon = { IconButton(onClick = { phones.removeAt(i) }) { Icon(Icons.Outlined.Close, stringResource(R.string.remove)) } },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
            TextButton(onClick = { phones.add(PhoneNumber(PhoneKind.MOBILE, "")) }) { Text(stringResource(R.string.add_number)) }
            if (original.id == null && store.contactPlaces.isNotEmpty()) {
                Choice(
                    stringResource(R.string.keep_in),
                    listOf("" to stringResource(R.string.on_this_phone)) + store.contactPlaces.map { it.accountId to it.address },
                    place,
                ) { place = it }
            }
            OutlinedTextField(notes, { notes = it }, label = { Text(stringResource(R.string.notes)) }, minLines = 3, modifier = Modifier.fillMaxWidth())
        }
    }
    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(stringResource(R.string.delete_contact_question)) },
            confirmButton = {
                TextButton(onClick = {
                    original.id?.let { store.deleteContact(it) }
                    confirmDelete = false
                    store.editingContact = null
                }) { Text(stringResource(R.string.delete)) }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text(stringResource(R.string.cancel)) } },
        )
    }
}
