@file:OptIn(ExperimentalMaterial3Api::class)

package nl.merel.app

import android.text.format.Formatter
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import uniffi.merel_mobile.MeaningState
import uniffi.merel_mobile.MeaningStatus
import uniffi.merel_mobile.meaningDownloadMegabytes

/** Search settings: search on meaning, a small language model on the phone itself. */
@Composable
fun SearchSettingsScreen(store: Store) {
    val h = LocalHelder.current
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { Text(stringResource(R.string.search_settings)) },
            navigationIcon = {
                IconButton(onClick = { store.screen = MailScreen.Folders }) {
                    Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back))
                }
            },
            colors = TopAppBarDefaults.topAppBarColors(containerColor = h.window),
        )
        Column(Modifier.verticalScroll(rememberScrollState()).padding(20.dp, 8.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Row(
                Modifier.fillMaxWidth().toggleable(store.searchOnMeaning, role = Role.Switch) { store.switchSearchOnMeaning(it) }.padding(vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(stringResource(R.string.search_on_meaning), style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                Switch(checked = store.searchOnMeaning, onCheckedChange = null)
            }
            Text(stringResource(R.string.search_on_meaning_hint, meaningDownloadMegabytes().toInt()), color = h.dim)
            store.meaningStatus?.takeIf { store.searchOnMeaning }?.let { MeaningStatusLine(store, it) }
        }
    }
}

@Composable
private fun MeaningStatusLine(store: Store, status: MeaningStatus) {
    val h = LocalHelder.current
    val context = LocalContext.current
    fun bytes(n: ULong) = Formatter.formatShortFileSize(context, n.toLong())
    when (val s = status.state) {
        is MeaningState.Downloading -> Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(stringResource(R.string.meaning_downloading))
            LinearProgressIndicator(
                progress = { if (s.total == 0uL) 0f else s.done.toFloat() / s.total.toFloat() },
                modifier = Modifier.fillMaxWidth(),
            )
            Text(stringResource(R.string.meaning_of, bytes(s.done), bytes(s.total)), color = h.dim, style = MaterialTheme.typography.bodySmall)
        }
        is MeaningState.Preparing -> Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.meaning_preparing), modifier = Modifier.weight(1f))
            CircularProgressIndicator()
        }
        is MeaningState.Ready -> Column {
            Text(stringResource(if (status.busy) R.string.meaning_reading else R.string.meaning_ready))
            Text(
                stringResource(R.string.meaning_read, status.messages.toInt(), bytes(status.modelBytes)),
                color = h.dim, style = MaterialTheme.typography.bodySmall,
            )
        }
        is MeaningState.Failed -> Column {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.WarningAmber, contentDescription = null, tint = h.flag)
                Spacer(Modifier.width(8.dp))
                Text(stringResource(R.string.meaning_failed), color = h.flag)
            }
            Text(s.reason, color = h.dim, style = MaterialTheme.typography.bodySmall)
            TextButton(onClick = { store.retryMeaning() }) { Text(stringResource(R.string.try_again)) }
        }
    }
}
