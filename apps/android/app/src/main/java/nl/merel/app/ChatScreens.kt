@file:OptIn(ExperimentalMaterial3Api::class)

package nl.merel.app

import android.content.Intent
import android.graphics.Bitmap
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Send
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import uniffi.merel_mobile.ChatMessageRow
import uniffi.merel_mobile.ChatRow
import uniffi.merel_mobile.CoreException

/** Which chat screen shows besides the list. */
private enum class ChatPage { List, New, Invite, Accept }

/**
 * Chat over e-mail, as on the computer: always end-to-end encrypted, and the other side can use
 * Merel or Delta Chat. The list of chats, a conversation, a new chat and invitations.
 */
@Composable
fun ChatsTab(store: Store) {
    var page by rememberSaveable { mutableStateOf(ChatPage.List) }
    store.openChatId?.let {
        Conversation(store, it)
        return
    }
    when (page) {
        ChatPage.New -> NewChat(store) { page = ChatPage.List }
        ChatPage.Invite -> Invite(store) { page = ChatPage.List }
        ChatPage.Accept -> AcceptInvite(store) { page = ChatPage.List }
        ChatPage.List -> ChatList(store) { page = it }
    }
}

@Composable
private fun ChatList(store: Store, go: (ChatPage) -> Unit) {
    val h = LocalHelder.current
    var menu by remember { mutableStateOf(false) }
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            TopAppBar(
                title = { Text(stringResource(R.string.tab_chat)) },
                actions = {
                    if (store.chatPlaces.isNotEmpty()) {
                        IconButton(onClick = { menu = true }) { Icon(Icons.Outlined.PersonAdd, stringResource(R.string.invite)) }
                        DropdownMenu(menu, { menu = false }) {
                            DropdownMenuItem({ Text(stringResource(R.string.my_invitation)) }, {
                                menu = false
                                go(ChatPage.Invite)
                            })
                            DropdownMenuItem({ Text(stringResource(R.string.open_invitation)) }, {
                                menu = false
                                go(ChatPage.Accept)
                            })
                        }
                    }
                },
                colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
            )
            if (store.chats.isEmpty()) {
                Box(Modifier.fillMaxSize().padding(32.dp), contentAlignment = Alignment.Center) {
                    Text(
                        stringResource(if (store.chatPlaces.isEmpty()) R.string.chat_needs_account else R.string.no_chats),
                        color = h.dim,
                    )
                }
            } else {
                LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
                    items(store.chats, key = { it.id }) { chat ->
                        ChatLine(chat, store.chatPlaces.size > 1) { store.openChat(chat.id) }
                        HorizontalDivider(color = h.border)
                    }
                }
            }
        }
        if (store.chatPlaces.isNotEmpty()) {
            FloatingActionButton(
                onClick = { go(ChatPage.New) },
                containerColor = h.accent,
                modifier = Modifier.align(Alignment.BottomEnd).padding(20.dp),
            ) { Icon(Icons.Outlined.Edit, stringResource(R.string.new_chat), tint = h.onAccent) }
        }
    }
}

/** Today the time, else the day. */
private fun shortDate(epoch: Long): String {
    val at = Instant.ofEpochSecond(epoch).atZone(ZoneId.systemDefault())
    return if (at.toLocalDate() == LocalDate.now()) {
        at.format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT))
    } else {
        at.format(DateTimeFormatter.ofPattern("d MMM"))
    }
}

@Composable
private fun ChatLine(chat: ChatRow, showAccount: Boolean, onClick: () -> Unit) {
    val h = LocalHelder.current
    val preview = when {
        chat.waiting -> stringResource(R.string.chat_waiting_short, chat.title)
        chat.preview == null -> stringResource(R.string.chat_encrypted)
        chat.lastMine -> stringResource(R.string.chat_mine, chat.preview!!)
        else -> chat.preview!!
    }
    Row(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Avatar(chat.title, chat.address, 44)
        Column(Modifier.padding(start = 12.dp).weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    chat.title, fontWeight = if (chat.unread > 0u) FontWeight.Bold else FontWeight.Normal,
                    maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
                )
                Text(shortDate(chat.last), color = h.dim, style = MaterialTheme.typography.bodySmall)
            }
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(preview, color = h.dim, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
                if (chat.unread > 0u) {
                    Text(
                        chat.unread.toString(), color = h.onAccent, style = MaterialTheme.typography.labelSmall,
                        modifier = Modifier.padding(start = 8.dp).clip(CircleShape).background(h.accent).padding(horizontal = 7.dp, vertical = 2.dp),
                    )
                }
            }
            if (showAccount) Text(chat.accountAddress, color = h.dim, style = MaterialTheme.typography.labelSmall)
        }
    }
}

/** One chat: the messages as bubbles, and a field to answer. */
@Composable
private fun Conversation(store: Store, id: String) {
    val h = LocalHelder.current
    BackHandler { store.openChat(null) }
    val chat = store.chats.firstOrNull { it.id == id }
    val messages = store.chatMessages
    var text by rememberSaveable { mutableStateOf("") }
    var note by remember { mutableStateOf<String?>(null) }
    var failed by remember { mutableStateOf(false) }
    val list = rememberLazyListState()
    val typeFirst = stringResource(R.string.type_message_first)
    val accepted = stringResource(R.string.invitation_accepted)
    LaunchedEffect(messages.size) { if (messages.isNotEmpty()) list.animateScrollToItem(messages.size - 1) }
    Column(Modifier.fillMaxSize().imePadding()) {
        TopAppBar(
            title = {
                Column {
                    Text(chat?.title ?: "", maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Outlined.Lock, contentDescription = null, tint = h.dim, modifier = Modifier.size(12.dp))
                        Text(" " + stringResource(R.string.encrypted), color = h.dim, style = MaterialTheme.typography.labelSmall)
                    }
                }
            },
            navigationIcon = { IconButton(onClick = { store.openChat(null) }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back)) } },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        LazyColumn(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 12.dp), state = list, verticalArrangement = Arrangement.spacedBy(6.dp)) {
            if (chat?.waiting == true) {
                item {
                    Text(stringResource(R.string.chat_waiting, chat.title), color = h.dim, modifier = Modifier.padding(16.dp))
                }
            }
            itemsIndexed(messages, key = { _, m -> m.id }) { i, m ->
                val first = i == 0 || messages[i - 1].senderAddress != m.senderAddress || messages[i - 1].mine != m.mine
                Bubble(m, showSender = first && !m.mine && chat?.isGroup == true) { link ->
                    val account = chat?.accountId ?: return@Bubble
                    store.acceptInvite(link, account)
                        .onSuccess {
                            note = accepted
                            failed = false
                        }
                        .onFailure { e ->
                            note = e.message
                            failed = true
                        }
                }
            }
        }
        note?.let { Text(it, color = if (failed) h.flag else h.dim, modifier = Modifier.padding(16.dp, 4.dp)) }
        if (chat?.waiting != true) {
            Row(Modifier.fillMaxWidth().background(h.window).padding(12.dp, 8.dp), verticalAlignment = Alignment.Bottom) {
                OutlinedTextField(
                    text, { text = it }, placeholder = { Text(stringResource(R.string.message)) }, maxLines = 6,
                    shape = RoundedCornerShape(20.dp), modifier = Modifier.weight(1f),
                )
                IconButton(
                    onClick = {
                        store.sendInChat(id, text)
                            .onSuccess {
                                text = ""
                                note = null
                            }
                            .onFailure { e ->
                                note = if (e is CoreException.Invalid) typeFirst else e.message
                                failed = true
                            }
                    },
                    enabled = text.isNotBlank(),
                ) { Icon(Icons.AutoMirrored.Outlined.Send, stringResource(R.string.send), tint = if (text.isNotBlank()) h.ink else h.dim) }
            }
        }
    }
}

@Composable
private fun Bubble(m: ChatMessageRow, showSender: Boolean, accept: (String) -> Unit) {
    val h = LocalHelder.current
    val time = Instant.ofEpochSecond(m.date).atZone(ZoneId.systemDefault()).format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT))
    Column(Modifier.fillMaxWidth(), horizontalAlignment = if (m.mine) Alignment.End else Alignment.Start) {
        if (showSender) Text(m.senderName, color = h.dim, style = MaterialTheme.typography.labelSmall)
        Column(
            Modifier.widthIn(max = 300.dp).clip(RoundedCornerShape(16.dp)).background(if (m.mine) h.selected else h.card)
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(m.text ?: stringResource(R.string.chat_opening), color = if (m.text == null) h.dim else h.text)
            val link = m.invite
            if (link != null && !m.mine) {
                Button(
                    onClick = { accept(link) },
                    colors = ButtonDefaults.buttonColors(containerColor = h.accent, contentColor = h.onAccent),
                ) { Text(stringResource(R.string.accept_invitation)) }
            }
            Text(time, color = h.dim, style = MaterialTheme.typography.labelSmall)
        }
    }
}

/** The account the chat goes from, when there is more than one. */
@Composable
private fun AccountChoice(store: Store, account: String, onChange: (String) -> Unit) {
    if (store.chatPlaces.size > 1) {
        Choice(stringResource(R.string.from), store.chatPlaces.map { it.accountId to it.address }, account, onChange)
    }
}

@Composable
private fun SubPage(title: Int, close: () -> Unit, content: @Composable () -> Unit) {
    val h = LocalHelder.current
    BackHandler(onBack = close)
    Column(Modifier.fillMaxSize().imePadding()) {
        TopAppBar(
            title = { Text(stringResource(title)) },
            navigationIcon = { IconButton(onClick = close) { Icon(Icons.Outlined.Close, stringResource(R.string.cancel)) } },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        Column(Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) { content() }
    }
}

/** A new chat: from which account, with whom, and the first message. */
@Composable
private fun NewChat(store: Store, close: () -> Unit) {
    val h = LocalHelder.current
    var account by rememberSaveable { mutableStateOf(store.chatPlaces.firstOrNull()?.accountId ?: "") }
    var people by rememberSaveable { mutableStateOf("") }
    var group by rememberSaveable { mutableStateOf(false) }
    var name by rememberSaveable { mutableStateOf("") }
    var text by rememberSaveable { mutableStateOf("") }
    val badAddress = stringResource(R.string.chat_bad_address)
    val typeFirst = stringResource(R.string.type_message_first)
    val typed = people.split(',').last().trim()
    val suggestions = if (typed.length >= 2) {
        store.contacts.filter { c -> c.name.contains(typed, true) || c.emails.any { it.contains(typed, true) } }.take(5)
    } else {
        emptyList()
    }
    SubPage(R.string.new_chat, close) {
        AccountChoice(store, account) { account = it }
        OutlinedTextField(
            people, { people = it }, label = { Text(stringResource(R.string.chat_people)) },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, autoCorrectEnabled = false), modifier = Modifier.fillMaxWidth(),
        )
        suggestions.forEach { c ->
            val email = c.emails.firstOrNull() ?: return@forEach
            Column(
                Modifier.fillMaxWidth().clickable {
                    people = (people.split(',').map { it.trim() }.dropLast(1) + email).joinToString(", ") + ", "
                }.padding(vertical = 6.dp),
            ) {
                Text(c.name)
                Text(email, color = h.dim, style = MaterialTheme.typography.bodySmall)
            }
        }
        Row(Modifier.fillMaxWidth().clickable { group = !group }, verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.group_chat), modifier = Modifier.weight(1f))
            Switch(group, { group = it })
        }
        if (group) {
            OutlinedTextField(name, { name = it }, label = { Text(stringResource(R.string.group_name)) }, singleLine = true, modifier = Modifier.fillMaxWidth())
        }
        OutlinedTextField(text, { text = it }, label = { Text(stringResource(R.string.first_message)) }, minLines = 3, modifier = Modifier.fillMaxWidth())
        Text(stringResource(R.string.chat_always_encrypted), color = h.dim, style = MaterialTheme.typography.bodySmall)
        Button(
            onClick = {
                val addresses = people.split(',', ';').map { it.trim() }.filter { it.isNotEmpty() }
                store.startChat(account, addresses, if (group) name else "", text)
                    .onSuccess { close() }
                    .onFailure { e ->
                        store.problem = when {
                            e is CoreException.Invalid && e.field == "people" -> badAddress
                            e is CoreException.Invalid -> typeFirst
                            else -> e.message
                        }
                    }
            },
            enabled = people.isNotBlank() && text.isNotBlank(),
            colors = ButtonDefaults.buttonColors(containerColor = h.accent, contentColor = h.onAccent),
            modifier = Modifier.fillMaxWidth(),
        ) { Text(stringResource(R.string.start_chat)) }
    }
}

/** The account's invitation: a QR code to scan and a link to share. */
@Composable
private fun Invite(store: Store, close: () -> Unit) {
    val h = LocalHelder.current
    val context = LocalContext.current
    var account by rememberSaveable { mutableStateOf(store.chatPlaces.firstOrNull()?.accountId ?: "") }
    val link = remember(account) { store.inviteLink(account) }
    val qr = remember(link) { link.getOrNull()?.let(::qrBitmap) }
    SubPage(R.string.my_invitation, close) {
        AccountChoice(store, account) { account = it }
        link.onFailure { Text(it.message ?: "", color = h.flag) }
        link.onSuccess { url ->
            qr?.let {
                Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                    Image(
                        it.asImageBitmap(), stringResource(R.string.invitation_qr), filterQuality = FilterQuality.None,
                        modifier = Modifier.size(240.dp).clip(RoundedCornerShape(12.dp)).background(Color.White).border(1.dp, h.border, RoundedCornerShape(12.dp)).padding(12.dp),
                    )
                }
            }
            Text(stringResource(R.string.invitation_hint), color = h.dim)
            Button(
                onClick = {
                    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, url)
                    context.startActivity(Intent.createChooser(send, null))
                },
                colors = ButtonDefaults.buttonColors(containerColor = h.accent, contentColor = h.onAccent),
                modifier = Modifier.fillMaxWidth(),
            ) { Text(stringResource(R.string.share_invitation)) }
        }
    }
}

/** A QR code of `text`, black on white, one pixel per module and a quiet zone. */
private fun qrBitmap(text: String): Bitmap? = runCatching {
    val matrix = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 0, 0)
    val bitmap = Bitmap.createBitmap(matrix.width, matrix.height, Bitmap.Config.ARGB_8888)
    for (x in 0 until matrix.width) for (y in 0 until matrix.height) {
        bitmap.setPixel(x, y, if (matrix[x, y]) android.graphics.Color.BLACK else android.graphics.Color.WHITE)
    }
    bitmap
}.getOrNull()

/** Accepts an invitation link someone sent, so a chat with them can start. */
@Composable
private fun AcceptInvite(store: Store, close: () -> Unit) {
    val h = LocalHelder.current
    var account by rememberSaveable { mutableStateOf(store.chatPlaces.firstOrNull()?.accountId ?: "") }
    var link by rememberSaveable { mutableStateOf("") }
    var done by rememberSaveable { mutableStateOf(false) }
    SubPage(R.string.open_invitation, close) {
        AccountChoice(store, account) { account = it }
        OutlinedTextField(
            link, { link = it }, label = { Text(stringResource(R.string.invitation_link)) },
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false), modifier = Modifier.fillMaxWidth(),
        )
        Text(stringResource(R.string.accept_hint), color = h.dim, style = MaterialTheme.typography.bodySmall)
        if (done) {
            Text(stringResource(R.string.invitation_accepted))
            TextButton(onClick = close) { Text(stringResource(R.string.done)) }
        } else {
            Button(
                onClick = {
                    store.acceptInvite(link, account).onSuccess { done = true }.onFailure { store.problem = it.message }
                },
                enabled = link.isNotBlank(),
                colors = ButtonDefaults.buttonColors(containerColor = h.accent, contentColor = h.onAccent),
                modifier = Modifier.fillMaxWidth(),
            ) { Text(stringResource(R.string.accept)) }
        }
        Spacer(Modifier.size(8.dp))
    }
}
