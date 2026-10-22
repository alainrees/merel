import SwiftUI

/// "Account toevoegen" for an IMAP/SMTP account, such as MXroute. Microsoft 365 and Gmail
/// sign in through the browser and follow later.
struct AddAccountView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss

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
            Form {
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
            .navigationTitle("Account toevoegen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Annuleren") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Toevoegen") { add() }
                        .disabled(busy || email.isEmpty || password.isEmpty || imapHost.isEmpty || smtpHost.isEmpty)
                }
            }
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

struct SecurityPicker: View {
    @Binding var selection: ConnectionSecurity

    var body: some View {
        Picker("Beveiliging", selection: $selection) {
            Text("TLS").tag(ConnectionSecurity.tls)
            Text("STARTTLS").tag(ConnectionSecurity.startTls)
        }
    }
}
