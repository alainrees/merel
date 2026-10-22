package nl.merel.app

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject
import uniffi.merel_mobile.AccountRow
import uniffi.merel_mobile.CoreException
import uniffi.merel_mobile.FolderRole
import uniffi.merel_mobile.FolderRow
import uniffi.merel_mobile.ImapAccount
import uniffi.merel_mobile.MerelCore
import uniffi.merel_mobile.MessageRow
import uniffi.merel_mobile.MessageView

/** Where the mail tab is: the folder list, a folder's messages, one message, or the account form. */
enum class MailScreen { Folders, Messages, Message, AddAccount }

/**
 * The app's state: Merel's engine plus what the screens show, refreshed when the engine's
 * background worker has news. Every change goes through a named engine command, as on the desktop.
 */
class Store(app: Application) : AndroidViewModel(app) {
    private val core: MerelCore

    var accounts by mutableStateOf(emptyList<AccountRow>())
        private set
    var folders by mutableStateOf(emptyList<FolderRow>())
        private set
    var messages by mutableStateOf(emptyList<MessageRow>())
        private set
    var openMessage by mutableStateOf<MessageView?>(null)
        private set
    var isSample by mutableStateOf(true)
        private set
    var filter by mutableStateOf("all")
        private set
    var screen by mutableStateOf(MailScreen.Messages)
    /** The last thing that went wrong, for a dialog; cleared when shown. */
    var problem by mutableStateOf<String?>(null)

    init {
        MerelNative.init(app)
        val dir = File(app.noBackupFilesDir, "merel").apply { mkdirs() }
        core = MerelCore(dir.path, KeystoreVault(app))
        refresh()
        folders.firstOrNull { it.role == FolderRole.INBOX }?.let { selectFolder(it.id) }
        viewModelScope.launch {
            while (true) {
                delay(500)
                if (core.poll()) refresh()
            }
        }
    }

    fun refresh() {
        accounts = core.accounts()
        folders = core.folders()
        messages = core.messages()
        openMessage = core.openMessage()
        isSample = core.isSample()
    }

    val selectedFolder: FolderRow? get() = folders.firstOrNull { it.selected }

    fun run(command: String, params: Map<String, Any> = emptyMap()) {
        try {
            core.run(command, JSONObject(params).toString())
        } catch (e: CoreException) {
            problem = e.message
        }
        refresh()
    }

    fun selectFolder(id: String) = run("folder.select", mapOf("id" to id))
    fun open(id: String) = run("message.select", mapOf("id" to id))
    fun refreshFolder() = run("folder.refresh")

    fun setFilter(value: String) {
        filter = value
        run("view.filter", mapOf("filter" to value))
    }

    /** Archives, deletes or flags one message from the list without opening it. */
    fun act(command: String, id: String) {
        run("message.select", mapOf("id" to id, "mark_read" to false))
        run(command)
    }

    /** Adds the account and opens its inbox; on failure, which field to check (or the reason). */
    fun addAccount(form: ImapAccount, password: String): Result<Unit> =
        try {
            core.addImapAccount(form, password)
            refresh()
            folders.firstOrNull { it.role == FolderRole.INBOX && it.accountId != "sample" }?.let { selectFolder(it.id) }
            screen = MailScreen.Messages
            Result.success(Unit)
        } catch (e: CoreException) {
            Result.failure(e)
        }

    override fun onCleared() {
        core.flush()
        core.close()
    }

    /** The app went to the background: write the offline cache now. */
    fun flush() = core.flush()
}
