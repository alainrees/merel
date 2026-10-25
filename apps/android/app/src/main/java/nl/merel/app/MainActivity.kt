package nl.merel.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CalendarMonth
import androidx.compose.material.icons.outlined.CheckBox
import androidx.compose.material.icons.outlined.Email
import androidx.compose.material.icons.outlined.Forum
import androidx.compose.material.icons.outlined.People
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource

class MainActivity : ComponentActivity() {
    private val store: Store by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        CustomTabSignIn.activity = java.lang.ref.WeakReference(this)
        setContent { MerelTheme { Root(store) } }
    }

    override fun onStop() {
        super.onStop()
        store.flush()
    }
}

/** The five modules along the bottom, as in the design. */
@Composable
fun Root(store: Store) {
    val h = LocalHelder.current
    var tab by rememberSaveable { mutableIntStateOf(0) }
    val tabs = listOf(
        Icons.Outlined.Email to R.string.tab_mail,
        Icons.Outlined.CalendarMonth to R.string.tab_calendar,
        Icons.Outlined.Forum to R.string.tab_chat,
        Icons.Outlined.People to R.string.tab_contacts,
        Icons.Outlined.CheckBox to R.string.tab_tasks,
    )
    Scaffold(
        containerColor = h.window,
        bottomBar = {
            NavigationBar(containerColor = h.card) {
                tabs.forEachIndexed { i, (icon, label) ->
                    NavigationBarItem(
                        selected = tab == i,
                        onClick = { tab = i },
                        icon = { Icon(icon, contentDescription = null) },
                        label = { Text(stringResource(label)) },
                        colors = NavigationBarItemDefaults.colors(
                            selectedIconColor = h.ink, selectedTextColor = h.ink, indicatorColor = h.selected,
                            unselectedIconColor = h.dim, unselectedTextColor = h.dim,
                        ),
                    )
                }
            }
        },
    ) { padding ->
        Box(Modifier.padding(padding).fillMaxSize()) {
            when (tab) {
                0 -> MailTab(store)
                1 -> CalendarTab(store)
                2 -> ChatsTab(store)
                3 -> ContactsTab(store)
                else -> TasksTab(store)
            }
        }
    }
    store.problem?.let { message ->
        AlertDialog(
            onDismissRequest = { store.problem = null },
            confirmButton = { TextButton(onClick = { store.problem = null }) { Text(stringResource(R.string.ok)) } },
            title = { Text(stringResource(R.string.failed)) },
            text = { Text(message) },
        )
    }
}
