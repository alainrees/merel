import CoreImage.CIFilterBuiltins
import SwiftUI

/// Chat over e-mail, as on the computer: always end-to-end encrypted, and the other side can
/// use Merel or Delta Chat. The list of chats, a conversation, a new chat and invitations.
struct ChatsTab: View {
    @EnvironmentObject private var store: Store
    @State private var path: [String] = []
    @State private var newChat = false
    @State private var invite = false
    @State private var accept = false

    var body: some View {
        NavigationStack(path: $path) {
            List {
                ForEach(store.chats, id: \.id) { chat in
                    NavigationLink(value: chat.id) {
                        ChatLine(chat: chat, showAccount: store.chatPlaces.count > 1)
                    }
                }
            }
            .listStyle(.plain)
            .overlay {
                if store.chats.isEmpty {
                    ContentUnavailableView(
                        "Nog geen chats", systemImage: "bubble.left.and.bubble.right",
                        description: Text(
                            store.chatPlaces.isEmpty
                                ? LocalizedStringKey("Chatten kan met een IMAP- of Microsoft 365-account. Voeg er een toe onder Mail.")
                                : LocalizedStringKey("Tik op het potlood om een chat te beginnen. Berichten zijn altijd versleuteld.")))
                }
            }
            .navigationTitle("Chat")
            .navigationDestination(for: String.self) { id in
                ConversationScreen(id: id)
            }
            .toolbar {
                if !store.chatPlaces.isEmpty {
                    ToolbarItem(placement: .topBarLeading) {
                        Menu {
                            Button("Mijn uitnodiging", systemImage: "qrcode") { invite = true }
                            Button("Uitnodiging openen", systemImage: "link") { accept = true }
                        } label: {
                            Label("Uitnodigen", systemImage: "person.crop.circle.badge.plus")
                        }
                    }
                    ToolbarItem(placement: .primaryAction) {
                        Button {
                            newChat = true
                        } label: {
                            Label("Nieuwe chat", systemImage: "square.and.pencil")
                        }
                    }
                }
            }
        }
        .sheet(isPresented: $newChat) {
            NewChatView { id in path = [id] }
        }
        .sheet(isPresented: $invite) {
            InviteView()
        }
        .sheet(isPresented: $accept) {
            AcceptInviteView()
        }
    }
}

struct ChatLine: View {
    let chat: ChatRow
    let showAccount: Bool

    var body: some View {
        HStack(spacing: 12) {
            Avatar(name: chat.title, address: chat.address, size: 44)
            VStack(alignment: .leading, spacing: 2) {
                HStack {
                    Text(chat.title).fontWeight(chat.unread > 0 ? .bold : .regular).lineLimit(1)
                    Spacer()
                    Text(shortDate(chat.last))
                        .font(.footnote).foregroundStyle(Helder.dim)
                }
                HStack {
                    Text(previewText).font(.subheadline).foregroundStyle(Helder.dim).lineLimit(2)
                    Spacer()
                    if chat.unread > 0 {
                        Text(verbatim: "\(chat.unread)")
                            .font(.caption.bold())
                            .padding(.horizontal, 7).padding(.vertical, 2)
                            .background(Helder.accent, in: Capsule())
                            .foregroundStyle(Helder.onAccent)
                            .accessibilityLabel(Text("\(Int(chat.unread)) ongelezen"))
                    }
                }
                if showAccount {
                    Text(chat.accountAddress).font(.caption).foregroundStyle(Helder.dim)
                }
            }
        }
        .padding(.vertical, 2)
    }

    private var previewText: String {
        if chat.waiting { return String(localized: "Wacht tot \(chat.title) de uitnodiging aanneemt") }
        guard let text = chat.preview else { return String(localized: "🔒 Versleuteld bericht") }
        return chat.lastMine ? String(localized: "Jij: \(text)") : text
    }
}

/// One chat: the messages as bubbles, and a field to answer.
struct ConversationScreen: View {
    @EnvironmentObject private var store: Store
    let id: String
    @State private var text = ""
    @State private var problem: String?
    @State private var accepted: String?

    private var chat: ChatRow? { store.chats.first { $0.id == id } }

    var body: some View {
        let messages = store.chatMessages(id)
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(spacing: 6) {
                    if chat?.waiting == true {
                        Text("Het eerste bericht gaat versleuteld weg zodra \(chat?.title ?? "") de uitnodiging heeft aangenomen.")
                            .font(.footnote).foregroundStyle(Helder.dim).multilineTextAlignment(.center)
                            .padding()
                    }
                    ForEach(Array(messages.enumerated()), id: \.element.id) { i, m in
                        let first = i == 0 || messages[i - 1].senderAddress != m.senderAddress || messages[i - 1].mine != m.mine
                        Bubble(message: m, showSender: first && !m.mine && chat?.isGroup == true) { link in
                            accept(link)
                        }
                        .id(m.id)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
            .defaultScrollAnchor(.bottom)
            .onChange(of: messages.last?.id) { _, last in
                if let last { withAnimation { proxy.scrollTo(last, anchor: .bottom) } }
            }
        }
        .background(Helder.window)
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 4) {
                if let problem {
                    Text(problem).font(.footnote).foregroundStyle(Helder.flag).frame(maxWidth: .infinity, alignment: .leading)
                }
                if let accepted {
                    Text(accepted).font(.footnote).foregroundStyle(Helder.dim).frame(maxWidth: .infinity, alignment: .leading)
                }
                if chat?.waiting != true {
                    HStack(alignment: .bottom, spacing: 8) {
                        TextField("Bericht", text: $text, axis: .vertical)
                            .lineLimit(1...6)
                            .padding(.horizontal, 12).padding(.vertical, 8)
                            .background(Helder.card, in: RoundedRectangle(cornerRadius: 18))
                            .overlay(RoundedRectangle(cornerRadius: 18).stroke(Helder.border))
                        Button {
                            send()
                        } label: {
                            Image(systemName: "arrow.up.circle.fill").font(.system(size: 32))
                        }
                        .disabled(text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityLabel(Text("Verzenden"))
                    }
                }
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .background(.bar)
        }
        .navigationTitle(chat?.title ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .principal) {
                VStack(spacing: 0) {
                    Text(chat?.title ?? "").font(.headline).lineLimit(1)
                    Label("Versleuteld", systemImage: "lock.fill").font(.caption2).foregroundStyle(Helder.dim)
                        .labelStyle(.titleAndIcon)
                }
            }
        }
        .onAppear { store.openChat(id) }
    }

    private func send() {
        problem = store.send(text, in: id)
        if problem == nil { text = "" }
    }

    private func accept(_ link: String) {
        guard let account = chat?.accountId else { return }
        if let failed = store.acceptInvite(link, on: account) {
            problem = failed
        } else {
            accepted = String(localized: "Uitnodiging aangenomen. De chat verschijnt zodra de ander heeft geantwoord.")
        }
    }
}

struct Bubble: View {
    let message: ChatMessageRow
    let showSender: Bool
    let accept: (String) -> Void

    var body: some View {
        HStack {
            if message.mine { Spacer(minLength: 48) }
            VStack(alignment: message.mine ? .trailing : .leading, spacing: 2) {
                if showSender {
                    Text(message.senderName).font(.caption).foregroundStyle(Helder.dim)
                }
                VStack(alignment: .leading, spacing: 6) {
                    if let text = message.text {
                        Text(text).textSelection(.enabled)
                    } else {
                        Label("Versleuteld bericht wordt geopend…", systemImage: "lock")
                            .foregroundStyle(Helder.dim)
                    }
                    if let link = message.invite, !message.mine {
                        Button("Uitnodiging aannemen") { accept(link) }
                            .buttonStyle(.borderedProminent)
                    }
                    Text(Date(timeIntervalSince1970: TimeInterval(message.date)), format: .dateTime.hour().minute())
                        .font(.caption2).foregroundStyle(Helder.dim)
                }
                .padding(.horizontal, 12).padding(.vertical, 8)
                .background(message.mine ? Helder.selected : Helder.card, in: RoundedRectangle(cornerRadius: 16))
            }
            if !message.mine { Spacer(minLength: 48) }
        }
        .accessibilityElement(children: .combine)
    }
}

/// A new chat: from which account, with whom (addresses or contacts), and the first message.
struct NewChatView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let opened: (String) -> Void
    @State private var account = ""
    @State private var people = ""
    @State private var group = false
    @State private var name = ""
    @State private var text = ""
    @State private var problem: String?

    /// Contacts whose address matches the last name being typed.
    private var suggestions: [(name: String, address: String)] {
        let typed = people.split(separator: ",").last.map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
        guard typed.count >= 2 else { return [] }
        return store.contacts(matching: typed).prefix(5).compactMap { c in c.emails.first.map { (c.name, $0) } }
    }

    var body: some View {
        NavigationStack {
            Form {
                if store.chatPlaces.count > 1 {
                    Picker("Van", selection: $account) {
                        ForEach(store.chatPlaces, id: \.accountId) { p in
                            Text(p.address).tag(p.accountId)
                        }
                    }
                }
                Section {
                    TextField("E-mailadressen, gescheiden door komma's", text: $people, axis: .vertical)
                        .keyboardType(.emailAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    ForEach(suggestions, id: \.address) { s in
                        Button {
                            var parts = people.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
                            if !parts.isEmpty { parts.removeLast() }
                            parts.append(s.address)
                            people = parts.joined(separator: ", ") + ", "
                        } label: {
                            VStack(alignment: .leading) {
                                Text(s.name)
                                Text(s.address).font(.footnote).foregroundStyle(Helder.dim)
                            }
                        }
                    }
                    Toggle("Groepschat", isOn: $group.animation())
                    if group {
                        TextField("Naam van de groep", text: $name)
                    }
                } header: {
                    Text("Met")
                }
                Section {
                    TextField("Eerste bericht", text: $text, axis: .vertical).lineLimit(3...8)
                } footer: {
                    Text("Chatberichten gaan altijd versleuteld. Heeft iemand nog geen sleutel, dan stuurt Merel eerst een uitnodiging en gaat je bericht weg zodra die is aangenomen.")
                }
                if let problem {
                    Section { Text(problem).foregroundStyle(Helder.flag) }
                }
            }
            .navigationTitle("Nieuwe chat")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Annuleer") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Start") { start() }
                        .disabled(people.trimmingCharacters(in: .whitespaces).isEmpty || text.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .onAppear { if account.isEmpty { account = store.chatPlaces.first?.accountId ?? "" } }
        }
    }

    private func start() {
        let addresses = people.split(whereSeparator: { $0 == "," || $0 == ";" }).map { $0.trimmingCharacters(in: .whitespaces) }
        switch store.startChat(from: account, with: addresses, name: group ? name : "", text: text) {
        case .success(let id):
            dismiss()
            opened(id)
        case .failure(let failure):
            problem = failure.message
        }
    }
}

/// The account's invitation: a QR code to scan and a link to share, for someone to start a
/// chat with you in Merel or Delta Chat.
struct InviteView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @State private var account = ""
    @State private var link: Result<String, ChatFailure>?

    var body: some View {
        NavigationStack {
            Form {
                if store.chatPlaces.count > 1 {
                    Picker("Account", selection: $account) {
                        ForEach(store.chatPlaces, id: \.accountId) { p in
                            Text(p.address).tag(p.accountId)
                        }
                    }
                }
                switch link {
                case .success(let url):
                    Section {
                        if let qr = qrImage(url) {
                            Image(uiImage: qr)
                                .interpolation(.none)
                                .resizable()
                                .scaledToFit()
                                .frame(maxWidth: 240)
                                .frame(maxWidth: .infinity)
                                .padding(12)
                                .background(Color.white, in: RoundedRectangle(cornerRadius: 12))
                                .accessibilityLabel(Text("QR-code van je uitnodiging"))
                        }
                        if let share = URL(string: url) {
                            ShareLink(item: share) {
                                Label("Uitnodiging delen", systemImage: "square.and.arrow.up")
                            }
                        }
                    } footer: {
                        Text("Laat de ander deze code scannen met Merel of Delta Chat, of deel de link. Daarna kunnen jullie versleuteld chatten.")
                    }
                case .failure(let failure):
                    Text(failure.message).foregroundStyle(Helder.flag)
                case nil:
                    ProgressView()
                }
            }
            .navigationTitle("Mijn uitnodiging")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Klaar") { dismiss() }
                }
            }
            .onAppear {
                if account.isEmpty { account = store.chatPlaces.first?.accountId ?? "" }
                link = store.inviteLink(account)
            }
            .onChange(of: account) { _, now in link = store.inviteLink(now) }
        }
    }

    private func qrImage(_ text: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
            let image = CIContext().createCGImage(output, from: output.extent)
        else { return nil }
        return UIImage(cgImage: image)
    }
}

/// Accepts an invitation link someone sent, so a chat with them can start.
struct AcceptInviteView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @State private var account = ""
    @State private var link = ""
    @State private var problem: String?
    @State private var done = false

    var body: some View {
        NavigationStack {
            Form {
                if store.chatPlaces.count > 1 {
                    Picker("Account", selection: $account) {
                        ForEach(store.chatPlaces, id: \.accountId) { p in
                            Text(p.address).tag(p.accountId)
                        }
                    }
                }
                Section {
                    TextField("https://i.delta.chat/#…", text: $link, axis: .vertical)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    PasteButton(payloadType: String.self) { strings in
                        if let first = strings.first { link = first }
                    }
                } footer: {
                    Text("Plak de uitnodigingslink die je kreeg. Een QR-code scan je met de Camera-app en kopieer je daarna.")
                }
                if let problem {
                    Section { Text(problem).foregroundStyle(Helder.flag) }
                }
                if done {
                    Section { Text("Uitnodiging aangenomen. De chat verschijnt zodra de ander heeft geantwoord.") }
                }
            }
            .navigationTitle("Uitnodiging openen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(done ? "Klaar" : "Annuleer") { dismiss() }
                }
                if !done {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Aannemen") {
                            problem = store.acceptInvite(link, on: account)
                            done = problem == nil
                        }
                        .disabled(link.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                }
            }
            .onAppear { if account.isEmpty { account = store.chatPlaces.first?.accountId ?? "" } }
        }
    }
}
