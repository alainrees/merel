@file:OptIn(ExperimentalMaterial3Api::class)

package nl.merel.app

import android.text.format.DateUtils
import androidx.activity.compose.BackHandler
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
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Archive
import androidx.compose.material.icons.outlined.AttachFile
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Drafts
import androidx.compose.material.icons.outlined.Flag
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Inbox
import androidx.compose.material.icons.outlined.MarkEmailUnread
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material.icons.outlined.Report
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.automirrored.outlined.Send
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import uniffi.merel_mobile.AccountRow
import uniffi.merel_mobile.ConnectionState
import uniffi.merel_mobile.SignInState
import uniffi.merel_mobile.FolderRole
import uniffi.merel_mobile.FolderRow
import uniffi.merel_mobile.MessageRow

@Composable
fun MailTab(store: Store) {
    when (store.screen) {
        MailScreen.Folders -> FolderScreen(store)
        MailScreen.Messages -> MessagesScreen(store)
        MailScreen.Message -> MessageScreen(store)
        MailScreen.AddAccount -> AddAccountScreen(store)
        MailScreen.SearchSettings -> SearchSettingsScreen(store)
    }
    BackHandler(enabled = store.screen != MailScreen.Messages) {
        store.screen = if (store.screen == MailScreen.SearchSettings) MailScreen.Folders else MailScreen.Messages
    }
}

@Composable
fun Avatar(name: String, address: String, size: Int = 40) {
    val text = initials(name).ifEmpty { initials(address) }
    Box(
        Modifier.size(size.dp).clip(CircleShape).background(avatarColor(address)).clearAndSetSemantics {},
        contentAlignment = Alignment.Center,
    ) {
        Text(text, color = Color.White, style = MaterialTheme.typography.bodyMedium)
    }
}

private fun Modifier.card(h: Helder) =
    this.clip(RoundedCornerShape(12.dp)).background(h.card).border(1.dp, h.border, RoundedCornerShape(12.dp))

// Folders

@Composable
fun FolderScreen(store: Store) {
    val h = LocalHelder.current
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(stringResource(R.string.folders)) },
            navigationIcon = {
                IconButton(onClick = { store.screen = MailScreen.Messages }) {
                    Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back))
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
            if (store.isSample) {
                item {
                    Text(stringResource(R.string.sample_hint), color = h.dim, modifier = Modifier.padding(4.dp, 8.dp))
                }
            }
            val open = { folder: FolderRow ->
                store.search("")
                store.selectFolder(folder.id)
                store.screen = MailScreen.Messages
            }
            if (store.allAccountsFolders.isNotEmpty()) {
                item(key = "head-all") {
                    Text(
                        stringResource(R.string.all_accounts), color = h.dim, style = MaterialTheme.typography.labelLarge,
                        modifier = Modifier.padding(4.dp, 16.dp, 4.dp, 6.dp).semantics { heading() },
                    )
                }
                items(store.allAccountsFolders, key = { it.id }) { folder -> FolderLine(folder, 0) { open(folder) } }
            }
            for (account in store.accounts) {
                item(key = "head-" + account.id) { AccountHeader(store, account) }
                items(tree(store.folders, account.id), key = { it.first.id }) { (folder, depth) ->
                    FolderLine(folder, depth) { open(folder) }
                }
            }
            item {
                TextButton(onClick = { store.screen = MailScreen.AddAccount }, modifier = Modifier.padding(top = 8.dp)) {
                    Icon(Icons.Outlined.PersonAdd, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.add_account))
                }
            }
            item {
                TextButton(onClick = { store.screen = MailScreen.SearchSettings }, modifier = Modifier.padding(bottom = 8.dp)) {
                    Icon(Icons.Outlined.Search, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.search_settings))
                }
            }
        }
    }
}

@Composable
private fun AccountHeader(store: Store, account: AccountRow) {
    val h = LocalHelder.current
    Row(Modifier.padding(4.dp, 16.dp, 4.dp, 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(account.address, color = h.dim, style = MaterialTheme.typography.labelLarge, modifier = Modifier.semantics { heading() })
        Spacer(Modifier.width(8.dp))
        when (val s = account.state) {
            is ConnectionState.Connecting -> Text(stringResource(R.string.connecting), color = h.dim, style = MaterialTheme.typography.labelMedium)
            is ConnectionState.Offline -> Text(stringResource(R.string.offline), color = h.dim, style = MaterialTheme.typography.labelMedium)
            is ConnectionState.Failed ->
                Icon(Icons.Outlined.WarningAmber, stringResource(R.string.account_problem) + ": " + s.reason, tint = h.flag)
            is ConnectionState.Online -> Unit
        }
        if (account.signsInWithBrowser && account.state is ConnectionState.Failed) {
            Spacer(Modifier.weight(1f))
            TextButton(onClick = { store.signInAgain(account.id) }, enabled = store.signIn !is SignInState.Waiting) {
                Text(stringResource(R.string.sign_in_again))
            }
        }
    }
}

@Composable
private fun FolderLine(folder: FolderRow, depth: Int, onClick: () -> Unit) {
    val h = LocalHelder.current
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp))
            .background(if (folder.selected) h.selected else Color.Transparent)
            .clickable(onClick = onClick).padding(start = (12 + depth * 18).dp, end = 12.dp, top = 12.dp, bottom = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(folderIcon(folder.role), contentDescription = null, tint = h.ink)
        Spacer(Modifier.width(14.dp))
        Text(folder.name, modifier = Modifier.weight(1f))
        if (folder.unread > 0u) {
            val label = stringResource(R.string.unread_count, folder.unread.toInt())
            Text(
                folder.unread.toString(), color = h.ink, fontWeight = FontWeight.Bold,
                modifier = Modifier.semantics { contentDescription = label },
            )
        }
    }
}

private fun folderIcon(role: FolderRole): ImageVector = when (role) {
    FolderRole.INBOX -> Icons.Outlined.Inbox
    FolderRole.DRAFTS -> Icons.Outlined.Drafts
    FolderRole.SENT -> Icons.AutoMirrored.Outlined.Send
    FolderRole.ARCHIVE -> Icons.Outlined.Archive
    FolderRole.JUNK -> Icons.Outlined.Report
    FolderRole.DELETED -> Icons.Outlined.Delete
    FolderRole.OTHER -> Icons.Outlined.Folder
}

/** An account's folders as a tree: special folders first in Outlook's order, then by name. */
private fun tree(all: List<FolderRow>, account: String): List<Pair<FolderRow, Int>> {
    val own = all.filter { it.accountId == account }
    val ids = own.map { it.id }.toSet()
    val out = mutableListOf<Pair<FolderRow, Int>>()
    fun walk(parent: String?, depth: Int) {
        own.filter { f -> f.parentId.let { p -> if (parent == null) p == null || p !in ids else p == parent } }
            .sortedWith(compareBy({ it.role.ordinal }, { it.name.lowercase() }))
            .forEach {
                out += it to depth
                if (depth < 6) walk(it.id, depth + 1)
            }
    }
    walk(null, 0)
    return out
}

// Message list

@Composable
fun MessagesScreen(store: Store) {
    val h = LocalHelder.current
    val query = store.searchWords
    val shown = store.messages
    val folder = store.selectedFolder
    // The folder of each row is worth showing when the list holds more than one folder.
    val showPlace = (query.isNotBlank() && store.allFolders) || folder?.accountId?.isEmpty() == true
    Column(Modifier.fillMaxSize()) {
        // The search bar of the design: the menu opens the folders, the avatar the accounts.
        Row(
            Modifier.padding(16.dp, 8.dp).fillMaxWidth().clip(RoundedCornerShape(28.dp)).background(h.card)
                .border(1.dp, h.border, RoundedCornerShape(28.dp)),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(onClick = { store.screen = MailScreen.Folders }) {
                Icon(Icons.Outlined.Menu, stringResource(R.string.open_folders))
            }
            TextField(
                value = query, onValueChange = { store.search(it) }, singleLine = true,
                placeholder = { Text(stringResource(R.string.search_mail)) },
                leadingIcon = { Icon(Icons.Outlined.Search, contentDescription = null) },
                colors = TextFieldDefaults.colors(
                    focusedContainerColor = Color.Transparent, unfocusedContainerColor = Color.Transparent,
                    focusedIndicatorColor = Color.Transparent, unfocusedIndicatorColor = Color.Transparent,
                ),
                modifier = Modifier.weight(1f),
            )
            store.accounts.firstOrNull { it.id != "sample" }?.let {
                Box(Modifier.padding(end = 8.dp).clickable { store.screen = MailScreen.Folders }) { Avatar(it.name, it.address, 34) }
            }
        }
        Column(Modifier.padding(20.dp, 4.dp)) {
            Text(
                folder?.name ?: stringResource(R.string.tab_mail), style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.Bold, modifier = Modifier.semantics { heading() },
            )
            val unread = store.messages.count { it.unread }
            if (unread > 0) Text(stringResource(R.string.unread_count, unread), color = h.dim, style = MaterialTheme.typography.bodyMedium)
        }
        LazyRow(Modifier.padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            item { Spacer(Modifier.width(8.dp)) }
            // Where a search looks: every folder of every account (the start), or this folder.
            if (query.isNotBlank()) {
                items(listOf(true to R.string.scope_all, false to R.string.scope_folder)) { (all, label) ->
                    FilterChip(
                        selected = store.allFolders == all, onClick = { store.search(query, all) }, label = { Text(stringResource(label)) },
                        colors = FilterChipDefaults.filterChipColors(selectedContainerColor = h.selected, selectedLabelColor = h.ink, containerColor = h.card),
                    )
                }
            }
            items(listOf("all" to R.string.filter_all, "unread" to R.string.filter_unread, "flagged" to R.string.filter_flagged, "attachments" to R.string.filter_attachments)) { (value, label) ->
                FilterChip(
                    selected = store.filter == value, onClick = { store.showOnly(value) }, label = { Text(stringResource(label)) },
                    colors = FilterChipDefaults.filterChipColors(selectedContainerColor = h.selected, selectedLabelColor = h.ink, containerColor = h.card),
                )
            }
        }
        PullToRefreshBox(isRefreshing = false, onRefresh = { store.refreshFolder() }, modifier = Modifier.fillMaxSize()) {
            if (shown.isEmpty()) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Text(stringResource(if (query.isEmpty()) R.string.no_messages else R.string.nothing_found), color = h.dim)
                }
            } else {
                LazyColumn(Modifier.fillMaxSize().padding(horizontal = 16.dp)) {
                    items(shown, key = { it.id }) { m ->
                        SwipeRow(store, m, showPlace)
                        HorizontalDivider(color = h.border)
                    }
                }
            }
        }
    }
}

/** Swipe left to delete, right to archive, as the design shows. */
@Composable
private fun SwipeRow(store: Store, m: MessageRow, showPlace: Boolean) {
    val h = LocalHelder.current
    val state = rememberSwipeToDismissBoxState(confirmValueChange = { value ->
        when (value) {
            SwipeToDismissBoxValue.EndToStart -> store.act("message.delete", m.id)
            SwipeToDismissBoxValue.StartToEnd -> store.act("message.archive", m.id)
            SwipeToDismissBoxValue.Settled -> Unit
        }
        false
    })
    SwipeToDismissBox(
        state = state,
        backgroundContent = {
            val toDelete = state.dismissDirection == SwipeToDismissBoxValue.EndToStart
            Row(
                Modifier.fillMaxSize().background(if (toDelete) Color(0xFFB3261E) else Color(0xFF555B63)).padding(horizontal = 20.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = if (toDelete) Arrangement.End else Arrangement.Start,
            ) {
                Icon(if (toDelete) Icons.Outlined.Delete else Icons.Outlined.Archive, contentDescription = null, tint = Color.White)
                Spacer(Modifier.width(8.dp))
                Text(stringResource(if (toDelete) R.string.delete else R.string.archive), color = Color.White)
            }
        },
    ) {
        MessageLine(m, showPlace, Modifier.background(h.card).clickable {
            store.open(m.id)
            store.screen = MailScreen.Message
        })
    }
}

@Composable
private fun MessageLine(m: MessageRow, showPlace: Boolean, modifier: Modifier) {
    val h = LocalHelder.current
    val unread = stringResource(R.string.unread)
    val flagged = stringResource(R.string.flagged)
    val attachment = stringResource(R.string.with_attachment)
    val subject = m.subject.ifEmpty { stringResource(R.string.no_subject) }
    val date = shortDate(m.date)
    // "Postvak IN · jij@example.org", with "Over hetzelfde" in front of a message found on meaning alone.
    val place = listOfNotNull(
        stringResource(R.string.related).takeIf { m.related },
        m.folderName.takeIf { showPlace },
        m.accountAddress.takeIf { showPlace && it.isNotEmpty() },
    ).joinToString(" · ")
    val summary = listOfNotNull(
        unread.takeIf { m.unread }, flagged.takeIf { m.flagged }, m.sender, subject, date, attachment.takeIf { m.hasAttachments }, m.preview,
        place.takeIf { it.isNotEmpty() },
    ).joinToString(", ")
    Row(modifier.fillMaxWidth().padding(12.dp, 11.dp).semantics(mergeDescendants = true) { contentDescription = summary }) {
        Box(Modifier.width(10.dp).padding(top = 16.dp)) {
            if (m.unread) Box(Modifier.size(8.dp).clip(CircleShape).background(h.accent))
        }
        Avatar(m.sender, m.senderAddress)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f).clearAndSetSemantics {}) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    m.sender, fontWeight = if (m.unread) FontWeight.Bold else FontWeight.Normal, maxLines = 1,
                    overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
                )
                if (m.hasAttachments) Icon(Icons.Outlined.AttachFile, null, tint = h.dim, modifier = Modifier.size(16.dp))
                if (m.flagged) Icon(Icons.Outlined.Flag, null, tint = h.flag, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(4.dp))
                Text(date, color = h.dim, style = MaterialTheme.typography.bodySmall)
            }
            Text(
                subject, color = if (m.unread) h.ink else h.text, fontWeight = if (m.unread) FontWeight.Bold else FontWeight.Normal,
                maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium,
            )
            Text(m.preview, color = h.dim, maxLines = 2, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyMedium)
            if (place.isNotEmpty()) {
                Text(place, color = h.dim, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

/** Today: the time; this week: the day; earlier: the date. In the phone's time zone and language. */
@Composable
fun shortDate(seconds: Long): String {
    val context = LocalContext.current
    val millis = seconds * 1000
    return when {
        DateUtils.isToday(millis) -> DateUtils.formatDateTime(context, millis, DateUtils.FORMAT_SHOW_TIME)
        DateUtils.isToday(millis + DateUtils.DAY_IN_MILLIS) -> stringResource(R.string.yesterday)
        System.currentTimeMillis() - millis < 6 * DateUtils.DAY_IN_MILLIS ->
            DateUtils.formatDateTime(context, millis, DateUtils.FORMAT_SHOW_WEEKDAY or DateUtils.FORMAT_ABBREV_WEEKDAY)
        else -> DateUtils.formatDateTime(context, millis, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH or DateUtils.FORMAT_NO_YEAR)
    }
}

// Reading

@Composable
fun MessageScreen(store: Store) {
    val h = LocalHelder.current
    val m = store.openMessage
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = {},
            navigationIcon = {
                IconButton(onClick = { store.screen = MailScreen.Messages }) {
                    Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back))
                }
            },
            actions = {
                if (m != null) {
                    IconButton(onClick = { store.run("message.archive"); store.screen = MailScreen.Messages }) {
                        Icon(Icons.Outlined.Archive, stringResource(R.string.archive))
                    }
                    IconButton(onClick = { store.run("message.delete"); store.screen = MailScreen.Messages }) {
                        Icon(Icons.Outlined.Delete, stringResource(R.string.delete))
                    }
                    IconButton(onClick = { store.run("message.mark_unread"); store.screen = MailScreen.Messages }) {
                        Icon(Icons.Outlined.MarkEmailUnread, stringResource(R.string.mark_unread))
                    }
                    IconButton(onClick = { store.run("message.flag") }) {
                        Icon(Icons.Outlined.Flag, stringResource(if (m.row.flagged) R.string.unflag else R.string.flag), tint = if (m.row.flagged) h.flag else h.text)
                    }
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        if (m == null) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Text(stringResource(R.string.no_message_open), color = h.dim) }
            return@Column
        }
        Column(Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text(
                m.row.subject.ifEmpty { stringResource(R.string.no_subject) }, style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.Bold, modifier = Modifier.semantics { heading() },
            )
            Row(Modifier.fillMaxWidth().card(h).padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
                Avatar(m.row.sender, m.row.senderAddress)
                Spacer(Modifier.width(12.dp))
                Column {
                    Text(m.row.sender, fontWeight = FontWeight.Bold)
                    Text(m.row.senderAddress, color = h.dim, style = MaterialTheme.typography.bodySmall)
                    Text(
                        DateUtils.formatDateTime(LocalContext.current, m.row.date * 1000, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_SHOW_TIME),
                        color = h.dim, style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
            val text = m.text
            if (text == null) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(10.dp))
                    Text(stringResource(R.string.fetching), color = h.dim)
                }
            } else {
                SelectionContainer { Text(text, style = MaterialTheme.typography.bodyLarge) }
            }
            for (name in m.attachments) {
                val label = stringResource(R.string.attachment, name)
                Row(
                    Modifier.fillMaxWidth().card(h).padding(10.dp).semantics(mergeDescendants = true) { contentDescription = label },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(Modifier.size(36.dp).clip(RoundedCornerShape(8.dp)).background(h.selected), contentAlignment = Alignment.Center) {
                        Icon(Icons.Outlined.Description, null, tint = h.ink)
                    }
                    Spacer(Modifier.width(10.dp))
                    Text(name)
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }
}
