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
    /// One up whenever the engine took in news, so the calendar asks for its appointments again.
    @Published private(set) var revision = 0
    /// What the search field looks for; empty when it is not searching.
    @Published private(set) var searchWords = ""
    @Published private(set) var searchOnMeaning = false
    @Published private(set) var meaningStatus: MeaningStatus?
    /// The last thing that went wrong, for an alert; cleared when shown.
    @Published var problem: String?
    private var timer: Timer?

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
        return Store(core: MerelCore(dataDir: dir.path, vault: KeychainVault()))
    }

    private func tick() {
        if core.poll() {
            refresh()
        } else if !searchWords.isEmpty {
            // The index and the language model answer a search a moment later.
            let found = core.messages()
            if found != messages { messages = found }
        }
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

    private func fieldName(_ field: String) -> String {
        switch field {
        case "email": String(localized: "het e-mailadres")
        case "username": String(localized: "de gebruikersnaam")
        case "password": String(localized: "het wachtwoord")
        case "incoming": String(localized: "de server voor inkomende mail")
        default: String(localized: "de server voor uitgaande mail")
        }
    }
}
