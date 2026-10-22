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
    @Published private(set) var messages: [MessageRow] = []
    @Published private(set) var openMessage: MessageView?
    @Published private(set) var isSample = true
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
        if core.poll() { refresh() }
    }

    func refresh() {
        accounts = core.accounts()
        folders = core.folders()
        messages = core.messages()
        openMessage = core.openMessage()
        isSample = core.isSample()
    }

    /// The app went to the background: write the offline cache now.
    func flush() {
        core.flush()
    }

    var selectedFolder: FolderRow? { folders.first(where: \.selected) }

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
