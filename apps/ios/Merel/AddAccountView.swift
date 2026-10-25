import SwiftUI

/// The kinds of account the phone adds.
enum AccountKind: Hashable {
    case microsoft, gmail, imap
}

/// "Account toevoegen": Microsoft 365 and Gmail sign in in the browser, any other account
/// (such as MXroute) with IMAP/SMTP and a password.
struct AddAccountView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    @State private var kind = AccountKind.microsoft

    @State private var name = ""
    @State private var email = ""
    @State private var password = ""
    @State private var username = ""
    @State private var imapHost = ""
    @State private var imapPort = "993"
    @State private var imapSecurity = ConnectionSecurity.tls
    @State private var smtpHost = ""
    @State private var smtpPort = "465"
    @State private var smtpSecurity = ConnectionSecurity.tls
    @State private var busy = false
    @State private var problem: String?

    var body: some View {
        NavigationStack {
            Group {
                if kind == .imap {
                    imapForm
                } else {
                    SignInForm(kind: kind, done: { dismiss() }) { kindPicker }
                }
            }
            .navigationTitle("Account toevoegen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Annuleren") {
                        store.clearSignIn()
                        dismiss()
                    }
                }
                if kind == .imap {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Toevoegen") { add() }
                            .disabled(busy || email.isEmpty || password.isEmpty || imapHost.isEmpty || smtpHost.isEmpty)
                    }
                }
            }
        }
    }

    private var kindPicker: some View {
        Section {
            Picker("Soort account", selection: $kind) {
                Text("Microsoft 365").tag(AccountKind.microsoft)
                Text("Gmail").tag(AccountKind.gmail)
                Text("Andere").tag(AccountKind.imap)
            }
            .pickerStyle(.segmented)
            .listRowBackground(Color.clear)
        }
    }

    private var imapForm: some View {
        Form {
            kindPicker
            Section {
                TextField("Naam", text: $name)
                    .textContentType(.name)
                TextField("E-mailadres", text: $email)
                    .textContentType(.emailAddress)
                    .keyboardType(.emailAddress)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onChange(of: email) { _, new in guessServers(from: new) }
                SecureField("Wachtwoord", text: $password)
                    .textContentType(.password)
            }
            Section("Inkomende mail (IMAP)") {
                TextField("Server", text: $imapHost)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                TextField("Poort", text: $imapPort).keyboardType(.numberPad)
                SecurityPicker(selection: $imapSecurity)
            }
            Section("Uitgaande mail (SMTP)") {
                TextField("Server", text: $smtpHost)
                    .textInputAutocapitalization(.never).autocorrectionDisabled().keyboardType(.URL)
                TextField("Poort", text: $smtpPort).keyboardType(.numberPad)
                SecurityPicker(selection: $smtpSecurity)
            }
            Section {
                TextField("Gebruikersnaam (als die anders is dan je adres)", text: $username)
                    .textInputAutocapitalization(.never).autocorrectionDisabled()
            } footer: {
                Text("Je wachtwoord blijft in de sleutelhanger van deze iPhone. Merel maakt alleen versleutelde verbindingen.")
            }
        }
        .alert(
            "Dat lukte niet",
            isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(problem ?? "")
        }
    }

    /// Fills in mail.<domain> as both servers, the most common setup, until the person types their own.
    private func guessServers(from address: String) {
        guard let domain = address.split(separator: "@").last, address.contains("@"), domain.contains(".") else { return }
        let guess = "mail.\(domain)"
        if imapHost.isEmpty || imapHost.hasPrefix("mail.") { imapHost = guess }
        if smtpHost.isEmpty || smtpHost.hasPrefix("mail.") { smtpHost = guess }
    }

    private func add() {
        busy = true
        defer { busy = false }
        let form = ImapAccount(
            name: name, email: email, username: username,
            incoming: ServerSettings(host: imapHost, port: UInt16(imapPort) ?? 0, security: imapSecurity),
            outgoing: ServerSettings(host: smtpHost, port: UInt16(smtpPort) ?? 0, security: smtpSecurity))
        problem = store.addAccount(form, password: password)
        if problem == nil {
            password = ""
            dismiss()
        }
    }
}

/// Name and address, for Gmail the person's own client ID and secret, and the button that
/// opens the provider's sign-in page; the password is typed there, never in Merel.
struct SignInForm<Header: View>: View {
    @EnvironmentObject private var store: Store
    let kind: AccountKind
    let done: () -> Void
    @ViewBuilder let picker: () -> Header
    @State private var name = ""
    @State private var email = ""
    @State private var clientId = ""
    @State private var clientSecret = ""
    @State private var problem: String?

    private var provider: String { kind == .gmail ? "Google" : "Microsoft" }

    var body: some View {
        Form {
            picker()
            Section {
                TextField("Naam", text: $name).textContentType(.name)
                TextField("E-mailadres", text: $email)
                    .textContentType(.emailAddress)
                    .keyboardType(.emailAddress)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
            } footer: {
                if kind == .microsoft {
                    Text("Je logt in bij Microsoft in Safari. Merel ziet je wachtwoord niet en bewaart alleen een sleutel om in te loggen, in de sleutelhanger van deze iPhone.")
                }
            }
            if kind == .gmail {
                Section {
                    TextField("Client-ID", text: $clientId)
                        .textInputAutocapitalization(.never).autocorrectionDisabled()
                    SecureField("Client-secret", text: $clientSecret)
                } footer: {
                    Text("Maak in de Google Cloud Console een OAuth-client van het type Desktop app en vul de client-ID en het client-secret in, dezelfde als op je computer.")
                }
            }
            Section {
                if store.signIn == .waiting {
                    HStack(spacing: 10) {
                        ProgressView()
                        Text("Wachten tot je bent ingelogd bij \(provider)…")
                    }
                    Button("Stoppen", role: .cancel) { store.clearSignIn() }
                } else {
                    Button("Inloggen bij \(provider)") { start() }
                        .disabled(email.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                if let problem {
                    Text(problem).foregroundStyle(Helder.flag)
                } else if case .failed(let reason) = store.signIn {
                    Text(reason).foregroundStyle(Helder.flag)
                }
            }
        }
        .onChange(of: store.signIn) { _, now in
            if now == .done {
                store.clearSignIn()
                done()
            }
        }
        .onChange(of: kind) { _, _ in
            problem = nil
            store.clearSignIn()
        }
    }

    private func start() {
        problem = store.signIn(
            kind == .gmail ? .gmail : .microsoft365, name: name, email: email,
            clientId: kind == .gmail ? clientId : "", clientSecret: kind == .gmail ? clientSecret : "")
    }
}

struct SecurityPicker: View {
    @Binding var selection: ConnectionSecurity

    var body: some View {
        Picker("Beveiliging", selection: $selection) {
            Text("TLS").tag(ConnectionSecurity.tls)
            Text("STARTTLS").tag(ConnectionSecurity.startTls)
        }
    }
}
