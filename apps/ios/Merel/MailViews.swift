import SwiftUI

/// Where the mail tab can be: a folder's messages, or one message.
enum MailRoute: Hashable {
    case folder(String)
    case message(String)
}

/// Mail: the folders, and on top of them the open folder's messages, as in the design.
struct MailTab: View {
    @EnvironmentObject private var store: Store
    @State private var path: [MailRoute] = []
    @State private var addingAccount = false

    var body: some View {
        NavigationStack(path: $path) {
            FolderList(addingAccount: $addingAccount)
                .navigationDestination(for: MailRoute.self) { route in
                    switch route {
                    case .folder(let id):
                        MessageList().onAppear { if store.selectedFolder?.id != id { store.selectFolder(id) } }
                    case .message(let id):
                        MessageScreen().onAppear { store.open(id) }
                    }
                }
        }
        .onAppear {
            if path.isEmpty, let folder = store.selectedFolder { path = [.folder(folder.id)] }
        }
        .sheet(isPresented: $addingAccount) {
            AddAccountView()
        }
    }
}

// MARK: - Folders

struct FolderList: View {
    @EnvironmentObject private var store: Store
    @Binding var addingAccount: Bool

    var body: some View {
        List {
            if store.isSample {
                Section {
                    Button {
                        addingAccount = true
                    } label: {
                        Label("Account toevoegen", systemImage: "person.badge.plus")
                    }
                } footer: {
                    Text("Je ziet nu voorbeeldmail. Voeg je eigen account toe om je mail te lezen.")
                }
            }
            if !store.allAccountsFolders.isEmpty {
                Section {
                    ForEach(store.allAccountsFolders, id: \.id) { folder in
                        NavigationLink(value: MailRoute.folder(folder.id)) {
                            FolderRowView(folder: folder, depth: 0)
                        }
                    }
                } header: {
                    Text("Alle accounts").textCase(nil)
                }
            }
            ForEach(store.accounts, id: \.id) { account in
                Section {
                    ForEach(tree(for: account.id), id: \.folder.id) { item in
                        NavigationLink(value: MailRoute.folder(item.folder.id)) {
                            FolderRowView(folder: item.folder, depth: item.depth)
                        }
                    }
                } header: {
                    AccountHeader(account: account)
                }
            }
            Section {
                if !store.isSample {
                    Button {
                        addingAccount = true
                    } label: {
                        Label("Account toevoegen", systemImage: "person.badge.plus")
                    }
                }
                NavigationLink {
                    SearchSettings()
                } label: {
                    Label("Zoeken", systemImage: "magnifyingglass")
                }
            }
        }
        .navigationTitle("Mappen")
    }

    /// The account's folders as a tree: special folders first in Outlook's order, then the
    /// others by name, each followed by its subfolders.
    private func tree(for account: String) -> [(folder: FolderRow, depth: Int)] {
        let own = store.folders.filter { $0.accountId == account }
        let ids = Set(own.map(\.id))
        func children(of parent: String?) -> [FolderRow] {
            own.filter { f in parent == nil ? (f.parentId.map { !ids.contains($0) } ?? true) : f.parentId == parent }
                .sorted { (order($0.role), $0.name.lowercased()) < (order($1.role), $1.name.lowercased()) }
        }
        var out: [(FolderRow, Int)] = []
        func walk(_ parent: String?, _ depth: Int) {
            for f in children(of: parent) {
                out.append((f, depth))
                if depth < 6 { walk(f.id, depth + 1) }
            }
        }
        walk(nil, 0)
        return out.map { (folder: $0.0, depth: $0.1) }
    }

    private func order(_ role: FolderRole) -> Int {
        switch role {
        case .inbox: 0
        case .drafts: 1
        case .sent: 2
        case .archive: 3
        case .junk: 4
        case .deleted: 5
        case .other: 6
        }
    }
}

struct AccountHeader: View {
    @EnvironmentObject private var store: Store
    let account: AccountRow

    var body: some View {
        HStack(spacing: 6) {
            Text(account.address)
            switch account.state {
            case .online: EmptyView()
            case .connecting: Text("Verbinden…").foregroundStyle(Helder.dim)
            case .offline: Text("Offline").foregroundStyle(Helder.dim)
            case .failed: Image(systemName: "exclamationmark.triangle").foregroundStyle(Helder.flag)
                .accessibilityLabel(Text("Probleem met dit account"))
            }
            if account.signsInWithBrowser, case .failed = account.state {
                Spacer()
                Button("Opnieuw aanmelden") { store.signInAgain(account.id) }
                    .font(.footnote)
                    .disabled(store.signIn == .waiting)
            }
        }
        .textCase(nil)
    }
}

struct FolderRowView: View {
    let folder: FolderRow
    let depth: Int

    var body: some View {
        HStack {
            Image(systemName: icon).foregroundStyle(Helder.ink).frame(width: 24)
                .accessibilityHidden(true)
            Text(folder.name)
            Spacer()
            if folder.unread > 0 {
                Text(verbatim: "\(folder.unread)").foregroundStyle(Helder.ink).fontWeight(.semibold)
                    .accessibilityLabel(Text("\(Int(folder.unread)) ongelezen"))
            }
        }
        .padding(.leading, CGFloat(depth) * 18)
    }

    private var icon: String {
        switch folder.role {
        case .inbox: "tray"
        case .drafts: "doc"
        case .sent: "paperplane"
        case .archive: "archivebox"
        case .junk: "xmark.shield"
        case .deleted: "trash"
        case .other: "folder"
        }
    }
}

// MARK: - Message list

struct MessageList: View {
    @EnvironmentObject private var store: Store
    @State private var query = ""
    @State private var composing: Composing?
    /// Where the search looks: every folder of every account (the start), or this folder.
    @State private var allFolders = true

    private var shown: [MessageRow] { store.messages }

    /// The folder of each row is worth showing when the list holds more than one folder.
    private var showPlace: Bool {
        (!store.searchWords.isEmpty && allFolders) || store.selectedFolder?.accountId.isEmpty == true
    }

    var body: some View {
        List {
            ForEach(shown, id: \.id) { m in
                NavigationLink(value: MailRoute.message(m.id)) {
                    MessageRowView(message: m, showPlace: showPlace)
                }
                .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                    Button(role: .destructive) {
                        store.act("message.delete", on: m.id)
                    } label: {
                        Label("Verwijderen", systemImage: "trash")
                    }
                    Button {
                        store.act("message.archive", on: m.id)
                    } label: {
                        Label("Archiveren", systemImage: "archivebox")
                    }
                    .tint(Color(rgb: 0x555B63))
                }
                .swipeActions(edge: .leading) {
                    Button {
                        store.act(m.unread ? "message.mark_read" : "message.mark_unread", on: m.id)
                    } label: {
                        Label(m.unread ? "Gelezen" : "Ongelezen", systemImage: m.unread ? "envelope.open" : "envelope.badge")
                    }
                    .tint(Helder.accent)
                    Button {
                        store.act("message.flag", on: m.id)
                    } label: {
                        Label(m.flagged ? "Markering weg" : "Markeren", systemImage: "flag")
                    }
                    .tint(Color(rgb: 0xB25E00))
                }
            }
        }
        .listStyle(.insetGrouped)
        .overlay {
            if shown.isEmpty {
                ContentUnavailableView(query.isEmpty ? "Geen berichten" : "Niets gevonden", systemImage: "tray")
            }
        }
        .searchable(text: $query, prompt: Text("Zoeken in mail"))
        .searchScopes($allFolders) {
            Text("Alle mappen").tag(true)
            Text("Deze map").tag(false)
        }
        .onChange(of: query) { store.search(query, allFolders: allFolders) }
        .onChange(of: allFolders) { store.search(query, allFolders: allFolders) }
        // A folder opened afresh shows all its mail; back from a message, the search stays.
        .onAppear { if query.isEmpty, !store.searchWords.isEmpty { store.search("", allFolders: allFolders) } }
        .refreshable { store.refreshFolder() }
        .navigationTitle(store.selectedFolder?.name ?? String(localized: "Mail"))
        .navigationBarTitleDisplayMode(.large)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    composing = Composing(form: store.startCompose(.new))
                } label: {
                    Label("Nieuw bericht", systemImage: "square.and.pencil")
                }
            }
        }
        .sheet(item: $composing) { item in
            ComposeView(form: item.form)
        }
    }
}

struct MessageRowView: View {
    let message: MessageRow
    var showPlace = false

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Avatar(name: message.sender, address: message.senderAddress)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 4) {
                    Text(message.sender).font(.body.weight(message.unread ? .bold : .regular)).lineLimit(1)
                    Spacer(minLength: 4)
                    if message.hasAttachments {
                        Image(systemName: "paperclip").foregroundStyle(Helder.dim).imageScale(.small)
                    }
                    if message.flagged {
                        Image(systemName: "flag.fill").foregroundStyle(Helder.flag).imageScale(.small)
                    }
                    Text(shortDate(message.date)).font(.footnote).foregroundStyle(Helder.dim)
                }
                Text(message.subject.isEmpty ? String(localized: "(geen onderwerp)") : message.subject)
                    .font(.subheadline.weight(message.unread ? .bold : .regular))
                    .foregroundStyle(message.unread ? Helder.ink : Helder.text)
                    .lineLimit(1)
                Text(message.preview).font(.subheadline).foregroundStyle(Helder.dim).lineLimit(2)
                if showPlace || message.related {
                    Text(place).font(.footnote).foregroundStyle(Helder.dim).lineLimit(1)
                }
            }
        }
        .padding(.vertical, 4)
        .overlay(alignment: .leading) {
            if message.unread {
                Circle().fill(Helder.accent).frame(width: 8, height: 8).offset(x: -14)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(accessibleSummary))
    }

    private var accessibleSummary: String {
        var parts: [String] = []
        if message.unread { parts.append(String(localized: "Ongelezen")) }
        if message.flagged { parts.append(String(localized: "Gemarkeerd")) }
        parts.append(message.sender)
        parts.append(message.subject)
        parts.append(shortDate(message.date))
        if message.hasAttachments { parts.append(String(localized: "met bijlage")) }
        parts.append(message.preview)
        if showPlace || message.related { parts.append(place) }
        return parts.joined(separator: ", ")
    }

    /// "Postvak IN · jij@example.org", with "Over hetzelfde" in front of a message found on
    /// meaning alone.
    private var place: String {
        var parts: [String] = []
        if message.related { parts.append(String(localized: "Over hetzelfde")) }
        if showPlace {
            parts.append(message.folderName)
            if !message.accountAddress.isEmpty { parts.append(message.accountAddress) }
        }
        return parts.joined(separator: " · ")
    }
}

/// Today: the time; this week: the day; earlier: the date. In the phone's time zone and language.
func shortDate(_ seconds: Int64) -> String {
    let date = Date(timeIntervalSince1970: TimeInterval(seconds))
    let cal = Calendar.current
    if cal.isDateInToday(date) { return date.formatted(date: .omitted, time: .shortened) }
    if cal.isDateInYesterday(date) { return String(localized: "gisteren") }
    if let days = cal.dateComponents([.day], from: date, to: .now).day, days < 7 {
        return date.formatted(.dateTime.weekday(.abbreviated))
    }
    return date.formatted(.dateTime.day().month(.abbreviated))
}

// MARK: - Reading

struct MessageScreen: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @State private var composing: Composing?

    var body: some View {
        Group {
            if let m = store.openMessage {
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        Text(m.row.subject.isEmpty ? String(localized: "(geen onderwerp)") : m.row.subject)
                            .font(.title2.bold())
                            .accessibilityAddTraits(.isHeader)
                        HStack(spacing: 12) {
                            Avatar(name: m.row.sender, address: m.row.senderAddress)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(m.row.sender).font(.headline)
                                Text(m.row.senderAddress).font(.footnote).foregroundStyle(Helder.dim)
                                Text(
                                    Date(timeIntervalSince1970: TimeInterval(m.row.date))
                                        .formatted(date: .abbreviated, time: .shortened)
                                ).font(.footnote).foregroundStyle(Helder.dim)
                            }
                            Spacer()
                        }
                        .padding(14)
                        .card()
                        if let text = m.text {
                            Text(text).font(.body).textSelection(.enabled)
                        } else {
                            ProgressView("Bericht ophalen…")
                        }
                        ForEach(m.attachments, id: \.self) { name in
                            HStack(spacing: 10) {
                                Image(systemName: "doc.text").foregroundStyle(Helder.ink)
                                    .frame(width: 36, height: 36)
                                    .background(Helder.selected, in: RoundedRectangle(cornerRadius: 8))
                                    .accessibilityHidden(true)
                                Text(name)
                                Spacer()
                            }
                            .padding(10)
                            .card()
                            .accessibilityLabel(Text("Bijlage \(name)"))
                        }
                    }
                    .padding(20)
                }
                .background(Helder.window)
                .toolbar {
                    ToolbarItemGroup(placement: .bottomBar) {
                        Button {
                            store.run("message.flag")
                        } label: {
                            Label(m.row.flagged ? "Markering weg" : "Markeren", systemImage: m.row.flagged ? "flag.fill" : "flag")
                        }
                        Spacer()
                        Menu {
                            Button("Beantwoorden", systemImage: "arrowshape.turn.up.left") { compose(.reply) }
                            Button("Allen beantwoorden", systemImage: "arrowshape.turn.up.left.2") { compose(.replyAll) }
                            Button("Doorsturen", systemImage: "arrowshape.turn.up.right") { compose(.forward) }
                        } label: {
                            Label("Beantwoorden", systemImage: "arrowshape.turn.up.left")
                        } primaryAction: {
                            compose(.reply)
                        }
                        Spacer()
                        Button {
                            store.run("message.archive")
                            dismiss()
                        } label: {
                            Label("Archiveren", systemImage: "archivebox")
                        }
                        Spacer()
                        Button {
                            store.run("message.delete")
                            dismiss()
                        } label: {
                            Label("Verwijderen", systemImage: "trash")
                        }
                    }
                }
            } else {
                ContentUnavailableView("Geen bericht geopend", systemImage: "envelope.open")
            }
        }
        .navigationBarTitleDisplayMode(.inline)
        .sheet(item: $composing) { item in
            ComposeView(form: item.form)
        }
    }

    private func compose(_ kind: ComposeKind) {
        composing = Composing(form: store.startCompose(kind))
    }
}
