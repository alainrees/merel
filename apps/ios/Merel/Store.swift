import Foundation
import SwiftUI

/// The app's state: Merel's engine plus what the screens show, refreshed when the engine's
/// background worker has news. Every change goes through a named engine command, as on the
/// desktop.
@MainActor
final class Store: ObservableObject {
    let core: MerelCore
    @Published private(set) var accounts: [AccountRow] = []
    @Published private(set) var folders: [FolderRow] = []
    /// "Alle accounts": Postvak IN and the other special folders of every account together.
    @Published private(set) var allAccountsFolders: [FolderRow] = []
    @Published private(set) var messages: [MessageRow] = []
    @Published private(set) var openMessage: MessageView?
    @Published private(set) var isSample = true
    @Published private(set) var calendars: [CalendarRow] = []
    @Published private(set) var tasks: [TaskRow] = []
    @Published private(set) var taskPlaces: [TaskPlace] = []
    @Published private(set) var allContacts: [ContactRow] = []
    @Published private(set) var contactPlaces: [TaskPlace] = []
    @Published private(set) var sendPlaces: [TaskPlace] = []
    /// Messages handed to the engine that have not gone out yet, to say so when one fails.
    private var sending: Set<UInt64> = []
    @Published private(set) var chats: [ChatRow] = []
    @Published private(set) var chatPlaces: [TaskPlace] = []
    /// Contact pictures already fetched, by contact id.
    private var photos: [String: Data] = [:]
    /// Whether the task list shows the finished tasks too.
    @Published var showDoneTasks = false {
        didSet { tasks = core.tasks(includeDone: showDoneTasks) }
    }
    /// One up whenever the engine took in news, so the calendar asks for its appointments again.
    @Published private(set) var revision = 0
    /// What the search field looks for; empty when it is not searching.
    @Published private(set) var searchWords = ""
    @Published private(set) var searchOnMeaning = false
    @Published private(set) var meaningStatus: MeaningStatus?
    /// Where a sign-in with Microsoft or Google is.
    @Published private(set) var signIn = SignInState.idle
    /// The last thing that went wrong, for an alert; cleared when shown.
    @Published var problem: String?
    private var timer: Timer?
    /// Whether the engine is busy with a poll off the main thread; the next tick waits for it.
    private var polling = false

    init(core: MerelCore) {
        self.core = core
        refresh()
        timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
        if let inbox = folders.first(where: { $0.role == .inbox }) {
            selectFolder(inbox.id)
        }
    }

    /// The engine on the app's own private folder, secrets in the keychain.
    static func live() -> Store {
        let fm = FileManager.default
        var dir = (fm.urls(for: .applicationSupportDirectory, in: .userDomainMask).first ?? fm.temporaryDirectory)
            .appendingPathComponent("Merel", isDirectory: true)
        try? fm.createDirectory(
            at: dir, withIntermediateDirectories: true,
            attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication])
        // The offline cache can be fetched again from the server; it stays out of backups.
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? dir.setResourceValues(values)
        setSignInBrowser(browser: SafariSignIn.shared)
        return Store(core: MerelCore(dataDir: dir.path, vault: KeychainVault()))
    }

    /// Polls the engine off the main thread: a poll can write the search index, which takes a
    /// moment the screen should not wait for. At utility priority, below the index's own
    /// threads, so the poll does not wait on lower-priority work.
    private func tick() {
        guard !polling else { return }
        polling = true
        let core = self.core
        Task.detached(priority: .utility) {
            let changed = core.poll()
            await MainActor.run {
                self.polling = false
                self.afterPoll(changed)
            }
        }
    }

    private func afterPoll(_ changed: Bool) {
        if changed {
            refresh()
        } else if !searchWords.isEmpty {
            // The index and the language model answer a search a moment later.
            let found = core.messages()
            if found != messages { messages = found }
        }
        if signIn == .waiting {
            followSignIn()
        }
        if !sending.isEmpty { followSending() }
        if searchOnMeaning {
            let status = core.meaningStatus()
            if status != meaningStatus { meaningStatus = status }
        }
    }

    func refresh() {
        accounts = core.accounts()
        folders = core.folders()
        allAccountsFolders = core.allAccountsFolders()
        searchOnMeaning = core.searchOnMeaning()
        meaningStatus = core.meaningStatus()
        messages = core.messages()
        openMessage = core.openMessage()
        isSample = core.isSample()
        calendars = core.calendars()
        tasks = core.tasks(includeDone: showDoneTasks)
        taskPlaces = core.taskPlaces()
        let contacts = core.contacts(query: "")
        if contacts != allContacts {
            allContacts = contacts
            photos = [:]
        }
        contactPlaces = core.contactPlaces()
        sendPlaces = core.sendPlaces()
        chats = core.chats()
        chatPlaces = core.chatPlaces()
        revision &+= 1
    }

    /// The app went to the background: write the offline cache now.
    func flush() {
        core.flush()
    }

    var selectedFolder: FolderRow? { allAccountsFolders.first(where: \.selected) ?? folders.first(where: \.selected) }

    // MARK: Search

    /// Searches every folder of every account, or the open folder only; empty words end the
    /// search.
    func search(_ words: String, allFolders: Bool) {
        let words = words.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "" : words
        searchWords = words
        core.search(words: words, allFolders: allFolders)
        messages = core.messages()
    }

    /// Switches search on meaning on (the language model is downloaded once) or off.
    func setSearchOnMeaning(_ on: Bool) {
        do {
            try core.setSearchOnMeaning(on: on)
        } catch {
            problem = error.localizedDescription
        }
        refresh()
    }

    func retryMeaning() {
        core.retryMeaning()
        refresh()
    }

    func run(_ command: String, _ params: [String: Any] = [:]) {
        let data = (try? JSONSerialization.data(withJSONObject: params)) ?? Data("{}".utf8)
        do {
            try core.run(command: command, paramsJson: String(decoding: data, as: UTF8.self))
        } catch {
            problem = error.localizedDescription
        }
        refresh()
    }

    func selectFolder(_ id: String) { run("folder.select", ["id": id]) }
    func open(_ id: String) { run("message.select", ["id": id]) }
    func refreshFolder() { run("folder.refresh") }

    /// Archives, deletes or flags one message from the list without opening it.
    func act(_ command: String, on id: String) {
        run("message.select", ["id": id, "mark_read": false])
        run(command)
    }

    /// Adds the account and opens its inbox; on failure, what to tell the person.
    func addAccount(_ form: ImapAccount, password: String) -> String? {
        do {
            _ = try core.addImapAccount(form: form, password: password)
            refresh()
            if let inbox = folders.first(where: { $0.role == .inbox && !$0.accountId.isEmpty && $0.accountId != "sample" }) {
                selectFolder(inbox.id)
            }
            return nil
        } catch CoreError.Invalid(let field) {
            return String(localized: "Controleer \(fieldName(field)).")
        } catch {
            return error.localizedDescription
        }
    }

    // MARK: Writing mail

    func startCompose(_ kind: ComposeKind) -> ComposeForm { core.startCompose(kind: kind) }

    /// Sends the message; on a form that is not right, what to tell the person. A failure on
    /// the way out comes later as `problem`.
    func sendMail(_ form: ComposeForm) -> String? {
        do {
            sending.insert(try core.sendMail(form: form))
            return nil
        } catch CoreError.Invalid(_) {
            return String(localized: "Kies het account waarmee je verstuurt.")
        } catch CoreError.Command(let reason) {
            return reason
        } catch {
            return error.localizedDescription
        }
    }

    private func followSending() {
        for id in sending {
            switch core.sendState(id: id) {
            case .sent:
                sending.remove(id)
            case .failed(let reason):
                sending.remove(id)
                problem = String(localized: "Het bericht is niet verstuurd: \(reason)")
            case .sending:
                break
            }
        }
    }

    // MARK: Chat

    func chatMessages(_ id: String) -> [ChatMessageRow] { core.chatMessages(id: id) }
    func openChat(_ id: String) { core.openChat(id: id) }

    /// Sends `text` in the chat; on failure, what to tell the person.
    func send(_ text: String, in chat: String) -> String? {
        do {
            try core.sendInChat(id: chat, text: text)
            refresh()
            return nil
        } catch {
            return chatProblem(error)
        }
    }

    /// Starts a chat and gives its id, or what to tell the person.
    func startChat(from account: String, with people: [String], name: String, text: String) -> Result<String, ChatFailure> {
        do {
            let id = try core.startChat(accountId: account, people: people, name: name, text: text)
            refresh()
            return .success(id)
        } catch {
            return .failure(ChatFailure(message: chatProblem(error)))
        }
    }

    func inviteLink(_ account: String) -> Result<String, ChatFailure> {
        do { return .success(try core.inviteLink(accountId: account)) } catch { return .failure(ChatFailure(message: chatProblem(error))) }
    }

    /// Accepts an invitation link; on failure, what to tell the person.
    func acceptInvite(_ link: String, on account: String) -> String? {
        do {
            _ = try core.acceptInvite(accountId: account, link: link)
            refresh()
            return nil
        } catch {
            return chatProblem(error)
        }
    }

    private func chatProblem(_ error: Error) -> String {
        switch error {
        case CoreError.Invalid(let field) where field == "people": String(localized: "Vul een geldig e-mailadres in.")
        case CoreError.Invalid(let field) where field == "text": String(localized: "Typ eerst een bericht.")
        case CoreError.Command(let reason): reason
        default: error.localizedDescription
        }
    }

    // MARK: Signing in with Microsoft or Google

    /// Opens the provider's sign-in page; on a form that is not right, what to tell the person.
    func signIn(
        _ provider: SignInProvider, name: String, email: String, clientId: String = "", clientSecret: String = ""
    ) -> String? {
        do {
            try core.signIn(provider: provider, name: name, email: email, clientId: clientId, clientSecret: clientSecret)
            followSignIn()
            return nil
        } catch CoreError.Invalid(let field) {
            return String(localized: "Controleer \(fieldName(field)).")
        } catch {
            return error.localizedDescription
        }
    }

    /// Signs an account in again, for one whose sign-in ran out.
    func signInAgain(_ accountId: String) {
        renewing = true
        core.signInAgain(accountId: accountId)
        signIn = .waiting
        followSignIn()
    }

    /// True while an existing account signs in again, outside "Account toevoegen".
    private var renewing = false

    /// Puts a finished or abandoned sign-in away.
    func clearSignIn() {
        core.clearSignIn()
        signIn = .idle
        SafariSignIn.shared.close()
    }

    private func followSignIn() {
        let now = core.signInState()
        guard now != signIn else { return }
        signIn = now
        switch now {
        case .done:
            SafariSignIn.shared.close()
            refresh()
        case .failed(let reason):
            SafariSignIn.shared.close()
            if renewing { problem = reason }
        default:
            break
        }
        if renewing, now != .waiting {
            renewing = false
            core.clearSignIn()
            signIn = .idle
        }
    }

    // MARK: Calendar

    /// The appointments touching `[from, to)`, a recurring one once per occurrence.
    func appointments(from: Date, to: Date) -> [AppointmentRow] {
        core.appointments(from: Int64(from.timeIntervalSince1970), to: Int64(to.timeIntervalSince1970))
    }

    func calendar(_ id: String) -> CalendarRow? { calendars.first { $0.id == id } }

    /// Saves the appointment; on failure, what to tell the person.
    func saveAppointment(_ form: AppointmentForm) -> String? {
        defer { refresh() }
        do {
            try core.saveAppointment(form: form)
            return nil
        } catch CoreError.Invalid(let field) {
            switch field {
            case "title": return String(localized: "Geef de afspraak een onderwerp.")
            case "end": return String(localized: "De afspraak moet eindigen na het begin.")
            case "recurrence": return String(localized: "Een terugkerende afspraak wijzig je voorlopig op de computer.")
            default: return String(localized: "Kies een agenda.")
            }
        } catch {
            return error.localizedDescription
        }
    }

    /// Deletes the appointment, or with `onlyThis` only this occurrence of a series.
    func deleteAppointment(_ a: AppointmentRow, onlyThis: Bool) {
        do {
            try core.deleteAppointment(id: a.id, occurrence: onlyThis ? a.start : nil)
        } catch {
            problem = error.localizedDescription
        }
        refresh()
    }

    // MARK: Contacts

    /// The contacts holding every word of `query`, by name.
    func contacts(matching query: String) -> [ContactRow] {
        query.trimmingCharacters(in: .whitespaces).isEmpty ? allContacts : core.contacts(query: query)
    }

    func contact(_ id: String) -> ContactRow? { allContacts.first { $0.id == id } }

    func contactPhoto(_ id: String) -> Data? {
        if let data = photos[id] { return data }
        let data = core.contactPhoto(id: id)
        photos[id] = data
        return data
    }

    /// Saves the contact; on failure, what to tell the person.
    func saveContact(_ form: ContactForm) -> String? {
        defer { refresh() }
        do {
            try core.saveContact(form: form)
            return nil
        } catch CoreError.Invalid(let field) {
            switch field {
            case "name": return String(localized: "Geef het contact een naam of e-mailadres.")
            case "emails": return String(localized: "Een van de e-mailadressen klopt niet.")
            default: return String(localized: "Dat lukte niet.")
            }
        } catch {
            return error.localizedDescription
        }
    }

    func deleteContact(_ id: String) {
        do {
            try core.deleteContact(id: id)
        } catch {
            problem = error.localizedDescription
        }
        refresh()
    }

    // MARK: Tasks

    /// Saves the task; on failure, what to tell the person.
    func saveTask(_ form: TaskForm) -> String? {
        defer { refresh() }
        do {
            try core.saveTask(form: form)
            return nil
        } catch CoreError.Invalid(let field) {
            switch field {
            case "title": return String(localized: "Geef de taak een naam.")
            case "repeat": return String(localized: "Een taak die terugkomt heeft een einddatum nodig.")
            default: return String(localized: "Dat lukte niet.")
            }
        } catch {
            return error.localizedDescription
        }
    }

    func completeTask(_ id: String, done: Bool) {
        do {
            try core.completeTask(id: id, done: done)
        } catch {
            problem = error.localizedDescription
        }
        refresh()
    }

    func deleteTask(_ id: String) {
        do {
            try core.deleteTask(id: id)
        } catch {
            problem = error.localizedDescription
        }
        refresh()
    }

    private func fieldName(_ field: String) -> String {
        switch field {
        case "email": String(localized: "het e-mailadres")
        case "username": String(localized: "de gebruikersnaam")
        case "password": String(localized: "het wachtwoord")
        case "incoming": String(localized: "de server voor inkomende mail")
        case "client_id": String(localized: "de client-ID")
        case "client_secret": String(localized: "het client-secret")
        default: String(localized: "de server voor uitgaande mail")
        }
    }
}

/// What went wrong with a chat, for a `Result`.
struct ChatFailure: Error {
    let message: String
}
