import SwiftUI

/// The message a sheet writes, with an identity for `sheet(item:)`.
struct Composing: Identifiable {
    let id = UUID()
    let form: ComposeForm
}

/// Nieuw bericht, Beantwoorden, Allen beantwoorden and Doorsturen: Van, Aan, CC/BCC with
/// suggestions from the contacts, Onderwerp and the text.
struct ComposeView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let original: ComposeForm
    @State private var account: String
    @State private var to: String
    @State private var cc: String
    @State private var bcc: String
    @State private var subject: String
    @State private var text: String
    @State private var showCopies: Bool
    @State private var problem: String?
    @State private var confirmDiscard = false
    @FocusState private var focus: Field?

    enum Field: Hashable { case to, cc, bcc, subject, body }

    init(form: ComposeForm) {
        original = form
        _account = State(initialValue: form.accountId)
        _to = State(initialValue: form.to)
        _cc = State(initialValue: form.cc)
        _bcc = State(initialValue: form.bcc)
        _subject = State(initialValue: form.subject)
        _text = State(initialValue: form.body)
        _showCopies = State(initialValue: !form.cc.isEmpty || !form.bcc.isEmpty)
    }

    private var changed: Bool {
        to != original.to || cc != original.cc || bcc != original.bcc || subject != original.subject || text != original.body
    }

    /// Contacts matching the address being typed in the focused field.
    private var suggestions: [(name: String, address: String)] {
        let field: String
        switch focus {
        case .to: field = to
        case .cc: field = cc
        case .bcc: field = bcc
        default: return []
        }
        let typed = field.split(whereSeparator: { $0 == "," || $0 == ";" }).last.map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
        guard typed.count >= 2, !field.hasSuffix(", ") else { return [] }
        return store.contacts(matching: typed).prefix(4).compactMap { c in c.emails.first.map { (c.name, $0) } }
    }

    var body: some View {
        NavigationStack {
            Form {
                if store.sendPlaces.count > 1 {
                    Picker("Van", selection: $account) {
                        ForEach(store.sendPlaces, id: \.accountId) { p in
                            Text(p.address).tag(p.accountId)
                        }
                    }
                }
                Section {
                    address("Aan", text: $to, field: .to)
                    if showCopies {
                        address("CC", text: $cc, field: .cc)
                        address("BCC", text: $bcc, field: .bcc)
                    } else {
                        Button("CC en BCC toevoegen") { showCopies = true }
                    }
                    ForEach(suggestions, id: \.address) { s in
                        Button {
                            pick(s.address)
                        } label: {
                            VStack(alignment: .leading) {
                                Text(s.name)
                                Text(s.address).font(.footnote).foregroundStyle(Helder.dim)
                            }
                        }
                    }
                    TextField("Onderwerp", text: $subject)
                        .focused($focus, equals: .subject)
                }
                Section {
                    TextField("Bericht", text: $text, axis: .vertical)
                        .lineLimit(10...)
                        .focused($focus, equals: .body)
                }
                if let problem {
                    Section { Text(problem).foregroundStyle(Helder.flag) }
                }
            }
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Annuleer") {
                        if changed { confirmDiscard = true } else { dismiss() }
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        send()
                    } label: {
                        Label("Verzenden", systemImage: "paperplane.fill")
                    }
                    .disabled(to.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .confirmationDialog("Dit bericht weggooien?", isPresented: $confirmDiscard, titleVisibility: .visible) {
                Button("Weggooien", role: .destructive) { dismiss() }
            }
            .interactiveDismissDisabled(changed)
            .onAppear {
                if account.isEmpty { account = store.sendPlaces.first?.accountId ?? "" }
                focus = to.isEmpty ? .to : .body
            }
        }
    }

    private var title: LocalizedStringKey {
        if original.inReplyTo != nil { return "Beantwoorden" }
        if !original.subject.isEmpty { return "Doorsturen" }
        return "Nieuw bericht"
    }

    private func address(_ label: LocalizedStringKey, text: Binding<String>, field: Field) -> some View {
        LabeledContent(label) {
            TextField("", text: text, axis: .vertical)
                .keyboardType(.emailAddress)
                .textContentType(.emailAddress)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .focused($focus, equals: field)
        }
    }

    /// Puts the picked address in place of what was typed, ready for the next one.
    private func pick(_ address: String) {
        func replaced(_ field: String) -> String {
            var parts = field.split(whereSeparator: { $0 == "," || $0 == ";" }).map { $0.trimmingCharacters(in: .whitespaces) }
            if !parts.isEmpty { parts.removeLast() }
            parts.append(address)
            return parts.joined(separator: ", ") + ", "
        }
        switch focus {
        case .to: to = replaced(to)
        case .cc: cc = replaced(cc)
        case .bcc: bcc = replaced(bcc)
        default: break
        }
    }

    private func send() {
        let form = ComposeForm(
            accountId: account, to: to, cc: cc, bcc: bcc, subject: subject, body: text, inReplyTo: original.inReplyTo)
        if let failed = store.sendMail(form) {
            problem = failed
        } else {
            dismiss()
        }
    }
}
