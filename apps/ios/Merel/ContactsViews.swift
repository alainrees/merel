import SwiftUI
import UIKit

/// Contacts: the list by name with search, a contact's card, and a form to add or change one.
struct ContactsTab: View {
    @EnvironmentObject private var store: Store
    @State private var query = ""
    @State private var editing: ContactForm?

    private var shown: [ContactRow] { store.contacts(matching: query) }

    var body: some View {
        NavigationStack {
            List {
                ForEach(shown, id: \.id) { c in
                    NavigationLink(value: c.id) {
                        HStack(spacing: 12) {
                            ContactPicture(contact: c, size: 40)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(c.name)
                                let line = subtitle(c)
                                if !line.isEmpty {
                                    Text(line).font(.footnote).foregroundStyle(Helder.dim).lineLimit(1)
                                }
                            }
                        }
                    }
                }
            }
            .listStyle(.plain)
            .overlay {
                if shown.isEmpty {
                    ContentUnavailableView(
                        query.isEmpty ? "Geen contacten" : "Niets gevonden", systemImage: "person.2",
                        description: query.isEmpty ? Text("Tik op + om een contact toe te voegen.") : nil)
                }
            }
            .searchable(text: $query, prompt: Text("Zoeken in contacten"))
            .navigationTitle("Contacten")
            .navigationDestination(for: String.self) { id in
                ContactScreen(id: id) { editing = $0 }
            }
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        editing = ContactForm(
                            id: nil, accountId: nil, name: "", company: "", jobTitle: "", emails: [""],
                            phones: [PhoneNumber(kind: .mobile, number: "")], notes: "")
                    } label: {
                        Label("Nieuw contact", systemImage: "plus")
                    }
                }
            }
        }
        .sheet(item: Binding(get: { editing.map(EditingContact.init) }, set: { editing = $0?.form })) { item in
            ContactEditor(form: item.form)
        }
    }

    private func subtitle(_ c: ContactRow) -> String {
        if !c.company.isEmpty { return c.company }
        return c.emails.first ?? ""
    }
}

/// The contact a sheet edits, with an identity for `sheet(item:)`.
struct EditingContact: Identifiable {
    let form: ContactForm
    var id: String { form.id ?? "new" }
}

/// The contact's photo, or its initials on a coloured circle.
struct ContactPicture: View {
    @EnvironmentObject private var store: Store
    let contact: ContactRow
    let size: CGFloat

    var body: some View {
        if contact.hasPhoto, let data = store.contactPhoto(contact.id), let image = UIImage(data: data) {
            Image(uiImage: image)
                .resizable()
                .scaledToFill()
                .frame(width: size, height: size)
                .clipShape(Circle())
                .accessibilityHidden(true)
        } else {
            Avatar(name: contact.name, address: contact.emails.first ?? contact.name, size: size)
        }
    }
}

/// A contact's card: picture, name, job, and every way to reach them.
struct ContactScreen: View {
    @EnvironmentObject private var store: Store
    @Environment(\.openURL) private var openURL
    let id: String
    let edit: (ContactForm) -> Void

    var body: some View {
        if let c = store.contact(id) {
            List {
                Section {
                    VStack(spacing: 8) {
                        ContactPicture(contact: c, size: 88)
                        Text(c.name).font(.title2.bold())
                        let job = [c.jobTitle, c.department, c.company].filter { !$0.isEmpty }.joined(separator: " · ")
                        if !job.isEmpty {
                            Text(job).foregroundStyle(Helder.dim).multilineTextAlignment(.center)
                        }
                        HStack(spacing: 12) {
                            if let email = c.emails.first, let url = URL(string: "mailto:\(email)") {
                                QuickAction(title: "Mail", systemImage: "envelope") { openURL(url) }
                            }
                            if let phone = c.phones.first, let url = telURL(phone.number) {
                                QuickAction(title: "Bellen", systemImage: "phone") { openURL(url) }
                            }
                        }
                        .padding(.top, 4)
                    }
                    .frame(maxWidth: .infinity)
                    .listRowBackground(Color.clear)
                }
                if !c.emails.isEmpty {
                    Section("E-mail") {
                        ForEach(c.emails, id: \.self) { email in
                            if let url = URL(string: "mailto:\(email)") {
                                Link(email, destination: url)
                            } else {
                                Text(email)
                            }
                        }
                    }
                }
                if !c.phones.isEmpty {
                    Section("Telefoon") {
                        ForEach(Array(c.phones.enumerated()), id: \.offset) { _, phone in
                            LabeledContent(phoneKindText(phone.kind)) {
                                if let url = telURL(phone.number) {
                                    Link(phone.number, destination: url)
                                } else {
                                    Text(phone.number)
                                }
                            }
                        }
                    }
                }
                if !c.addresses.isEmpty {
                    Section("Adres") {
                        ForEach(Array(c.addresses.enumerated()), id: \.offset) { _, a in
                            LabeledContent(a.work ? String(localized: "Werk") : a.home ? String(localized: "Thuis") : String(localized: "Overig")) {
                                Text(a.lines.joined(separator: "\n")).multilineTextAlignment(.trailing)
                            }
                        }
                    }
                }
                if c.birthday != nil || !c.website.isEmpty {
                    Section {
                        if let birthday = c.birthday {
                            LabeledContent("Verjaardag", value: birthdayText(birthday))
                        }
                        if !c.website.isEmpty {
                            LabeledContent("Website", value: c.website)
                        }
                    }
                }
                if !c.notes.isEmpty {
                    Section("Notities") {
                        Text(c.notes).textSelection(.enabled)
                    }
                }
                Section {
                    LabeledContent("Bewaard in", value: c.accountAddress ?? String(localized: "Op deze telefoon"))
                }
            }
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("Wijzig") {
                        edit(
                            ContactForm(
                                id: c.id, accountId: c.accountId, name: c.name, company: c.company, jobTitle: c.jobTitle,
                                emails: c.emails.isEmpty ? [""] : c.emails, phones: c.phones, notes: c.notes))
                    }
                }
            }
        } else {
            ContentUnavailableView("Dit contact is er niet meer", systemImage: "person.crop.circle.badge.xmark")
        }
    }

    private func telURL(_ number: String) -> URL? {
        let digits = number.filter { $0.isNumber || $0 == "+" }
        return digits.isEmpty ? nil : URL(string: "tel:\(digits)")
    }

    /// "12 maart 1985", or "12 maart" for a birthday without a year.
    private func birthdayText(_ iso: String) -> String {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd"
        f.timeZone = TimeZone(identifier: "UTC")
        guard let date = f.date(from: iso) else { return iso }
        return date.formatted(.dateTime.day().month(.wide).year().locale(.current))
    }
}

struct QuickAction: View {
    let title: LocalizedStringKey
    let systemImage: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 4) {
                Image(systemName: systemImage).font(.title3)
                Text(title).font(.caption)
            }
            .frame(width: 72, height: 56)
            .background(Helder.card, in: RoundedRectangle(cornerRadius: 12))
        }
        .buttonStyle(.plain)
        .foregroundStyle(Helder.ink)
    }
}

func phoneKindText(_ kind: PhoneKind) -> String {
    switch kind {
    case .mobile: String(localized: "Mobiel")
    case .work: String(localized: "Werk")
    case .home: String(localized: "Thuis")
    case .other: String(localized: "Overig")
    }
}

struct ContactEditor: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let original: ContactForm
    @State private var name: String
    @State private var company: String
    @State private var jobTitle: String
    @State private var emails: [String]
    @State private var phones: [PhoneNumber]
    @State private var notes: String
    @State private var place: String
    @State private var problem: String?
    @State private var confirmDelete = false

    init(form: ContactForm) {
        original = form
        _name = State(initialValue: form.name)
        _company = State(initialValue: form.company)
        _jobTitle = State(initialValue: form.jobTitle)
        _emails = State(initialValue: form.emails)
        _phones = State(initialValue: form.phones)
        _notes = State(initialValue: form.notes)
        _place = State(initialValue: form.accountId ?? "")
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Naam", text: $name).textContentType(.name)
                    TextField("Bedrijf", text: $company).textContentType(.organizationName)
                    TextField("Functie", text: $jobTitle).textContentType(.jobTitle)
                }
                Section("E-mail") {
                    ForEach(emails.indices, id: \.self) { i in
                        TextField("E-mailadres", text: $emails[i])
                            .keyboardType(.emailAddress)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                    }
                    .onDelete { emails.remove(atOffsets: $0) }
                    Button("E-mailadres toevoegen") { emails.append("") }
                }
                Section("Telefoon") {
                    ForEach(phones.indices, id: \.self) { i in
                        HStack {
                            Picker("Soort", selection: $phones[i].kind) {
                                ForEach([PhoneKind.mobile, .work, .home, .other], id: \.self) { k in
                                    Text(phoneKindText(k)).tag(k)
                                }
                            }
                            .labelsHidden()
                            .fixedSize()
                            TextField("Nummer", text: $phones[i].number).keyboardType(.phonePad)
                        }
                    }
                    .onDelete { phones.remove(atOffsets: $0) }
                    Button("Nummer toevoegen") { phones.append(PhoneNumber(kind: .mobile, number: "")) }
                }
                if original.id == nil, !store.contactPlaces.isEmpty {
                    Section {
                        Picker("Bewaren in", selection: $place) {
                            Text("Op deze telefoon").tag("")
                            ForEach(store.contactPlaces, id: \.accountId) { p in
                                Text(p.address).tag(p.accountId)
                            }
                        }
                    }
                }
                Section("Notities") {
                    TextField("Notities", text: $notes, axis: .vertical).lineLimit(3...10)
                }
                if let problem {
                    Section {
                        Text(problem).foregroundStyle(Helder.flag)
                    }
                }
                if original.id != nil {
                    Section {
                        Button("Contact verwijderen", role: .destructive) { confirmDelete = true }
                    }
                }
            }
            .navigationTitle(original.id == nil ? "Nieuw contact" : "Contact wijzigen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Annuleer") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Bewaar") { save() }
                        .disabled(name.trimmingCharacters(in: .whitespaces).isEmpty && emails.allSatisfy { $0.trimmingCharacters(in: .whitespaces).isEmpty })
                }
            }
            .confirmationDialog("Dit contact verwijderen?", isPresented: $confirmDelete, titleVisibility: .visible) {
                Button("Verwijderen", role: .destructive) {
                    if let id = original.id { store.deleteContact(id) }
                    dismiss()
                }
            }
        }
    }

    private func save() {
        let form = ContactForm(
            id: original.id, accountId: place.isEmpty ? nil : place, name: name, company: company, jobTitle: jobTitle,
            emails: emails, phones: phones, notes: notes)
        if let message = store.saveContact(form) {
            problem = message
        } else {
            dismiss()
        }
    }
}
