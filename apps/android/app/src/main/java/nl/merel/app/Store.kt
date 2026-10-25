package nl.merel.app

import android.app.Application
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.time.LocalDate
import java.time.ZoneId
import uniffi.merel_mobile.AccountRow
import uniffi.merel_mobile.AppointmentForm
import uniffi.merel_mobile.AppointmentRow
import uniffi.merel_mobile.CalendarRow
import uniffi.merel_mobile.ContactForm
import uniffi.merel_mobile.ContactRow
import uniffi.merel_mobile.CoreException
import uniffi.merel_mobile.FolderRole
import uniffi.merel_mobile.FolderRow
import uniffi.merel_mobile.ImapAccount
import uniffi.merel_mobile.MeaningStatus
import uniffi.merel_mobile.MerelCore
import uniffi.merel_mobile.MessageRow
import uniffi.merel_mobile.MessageView
import uniffi.merel_mobile.TaskForm
import uniffi.merel_mobile.TaskPlace
import uniffi.merel_mobile.TaskRow

/** Where the mail tab is: the folder list, a folder's messages, one message, or the account form. */
enum class MailScreen { Folders, Messages, Message, AddAccount, SearchSettings }

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
    /** "Alle accounts": the inbox and the other special folders of every account together. */
    var allAccountsFolders by mutableStateOf(emptyList<FolderRow>())
        private set
    /** What the search field holds, and whether it looks in every folder or the open one. */
    var searchWords by mutableStateOf("")
        private set
    var allFolders by mutableStateOf(true)
        private set
    var searchOnMeaning by mutableStateOf(false)
        private set
    var meaningStatus by mutableStateOf<MeaningStatus?>(null)
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
    var calendars by mutableStateOf(emptyList<CalendarRow>())
        private set
    /** One up whenever the engine took in news, so the calendar asks for its appointments again. */
    var revision by mutableIntStateOf(0)
        private set
    /** The calendar's chosen day, the appointment open on top of it, and the form being edited. */
    var day by mutableStateOf(LocalDate.now())
    var openAppointment by mutableStateOf<Pair<String, Long>?>(null)
    var editing by mutableStateOf<AppointmentForm?>(null)
    var tasks by mutableStateOf(emptyList<TaskRow>())
        private set
    var taskPlaces by mutableStateOf(emptyList<TaskPlace>())
        private set
    /** Whether the task list shows the finished tasks too, and the task form being edited. */
    var showDoneTasks by mutableStateOf(false)
        private set
    var editingTask by mutableStateOf<TaskForm?>(null)
    var contacts by mutableStateOf(emptyList<ContactRow>())
        private set
    var contactPlaces by mutableStateOf(emptyList<TaskPlace>())
        private set
    /** What the contact search holds, the contact open on its card, and the form being edited. */
    var contactQuery by mutableStateOf("")
        private set
    var openContact by mutableStateOf<String?>(null)
    var editingContact by mutableStateOf<ContactForm?>(null)
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
                if (core.poll()) {
                    refresh()
                } else if (searchWords.isNotBlank()) {
                    // The index and the language model answer a search a moment later.
                    val found = core.messages()
                    if (found != messages) messages = found
                }
                if (searchOnMeaning) {
                    val status = core.meaningStatus()
                    if (status != meaningStatus) meaningStatus = status
                }
            }
        }
    }

    fun refresh() {
        accounts = core.accounts()
        folders = core.folders()
        allAccountsFolders = core.allAccountsFolders()
        searchOnMeaning = core.searchOnMeaning()
        meaningStatus = core.meaningStatus()
        messages = core.messages()
        openMessage = core.openMessage()
        isSample = core.isSample()
        calendars = core.calendars()
        tasks = core.tasks(showDoneTasks)
        taskPlaces = core.taskPlaces()
        contacts = core.contacts(contactQuery)
        contactPlaces = core.contactPlaces()
        revision++
    }

    val selectedFolder: FolderRow? get() = allAccountsFolders.firstOrNull { it.selected } ?: folders.firstOrNull { it.selected }

    /** Searches every folder of every account, or with [all] false the open folder only. */
    fun search(words: String, all: Boolean = allFolders) {
        searchWords = words
        allFolders = all
        core.search(if (words.isBlank()) "" else words, all)
        messages = core.messages()
    }

    /** Switches search on meaning on (the language model is downloaded once) or off. */
    fun switchSearchOnMeaning(on: Boolean) {
        try {
            core.setSearchOnMeaning(on)
        } catch (e: CoreException) {
            problem = e.message
        }
        refresh()
    }

    fun retryMeaning() {
        core.retryMeaning()
        refresh()
    }

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

    fun showOnly(value: String) {
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

    /** The appointments of `[from, to)` on the phone's clock, a recurring one once per occurrence. */
    fun appointments(from: LocalDate, to: LocalDate): List<AppointmentRow> {
        val zone = ZoneId.systemDefault()
        return core.appointments(from.atStartOfDay(zone).toEpochSecond(), to.atStartOfDay(zone).toEpochSecond())
    }

    fun calendar(id: String): CalendarRow? = calendars.firstOrNull { it.id == id }

    /** Saves the appointment; on failure, which field was wrong (or the reason). */
    fun saveAppointment(form: AppointmentForm): Result<Unit> =
        try {
            core.saveAppointment(form)
            Result.success(Unit)
        } catch (e: CoreException) {
            Result.failure(e)
        } finally {
            refresh()
        }

    /** Deletes the appointment, or with [onlyThis] only this occurrence of a series. */
    fun deleteAppointment(a: AppointmentRow, onlyThis: Boolean) {
        try {
            core.deleteAppointment(a.id, if (onlyThis) a.start else null)
        } catch (e: CoreException) {
            problem = e.message
        }
        openAppointment = null
        refresh()
    }

    fun showDone(on: Boolean) {
        showDoneTasks = on
        tasks = core.tasks(on)
    }

    /** Saves the task; on failure, which field was wrong (or the reason). */
    fun saveTask(form: TaskForm): Result<Unit> =
        try {
            core.saveTask(form)
            Result.success(Unit)
        } catch (e: CoreException) {
            Result.failure(e)
        } finally {
            refresh()
        }

    fun completeTask(id: String, done: Boolean) {
        try {
            core.completeTask(id, done)
        } catch (e: CoreException) {
            problem = e.message
        }
        refresh()
    }

    fun deleteTask(id: String) {
        try {
            core.deleteTask(id)
        } catch (e: CoreException) {
            problem = e.message
        }
        refresh()
    }

    fun searchContacts(query: String) {
        contactQuery = query
        contacts = core.contacts(query)
    }

    fun contact(id: String): ContactRow? = core.contacts("").firstOrNull { it.id == id }

    fun contactPhoto(id: String): ByteArray? = core.contactPhoto(id)

    /** Saves the contact; on failure, which field was wrong (or the reason). */
    fun saveContact(form: ContactForm): Result<Unit> =
        try {
            core.saveContact(form)
            Result.success(Unit)
        } catch (e: CoreException) {
            Result.failure(e)
        } finally {
            refresh()
        }

    fun deleteContact(id: String) {
        try {
            core.deleteContact(id)
        } catch (e: CoreException) {
            problem = e.message
        }
        openContact = null
        refresh()
    }

    override fun onCleared() {
        core.flush()
        core.close()
    }

    /** The app went to the background: write the offline cache now. */
    fun flush() = core.flush()
}
