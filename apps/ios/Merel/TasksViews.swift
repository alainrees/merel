import SwiftUI

/// Tasks: the open ones by due day, a tap to finish one, and a form to add or change one.
struct TasksTab: View {
    @EnvironmentObject private var store: Store
    @State private var editing: TaskForm?

    var body: some View {
        NavigationStack {
            List {
                ForEach(groups) { group in
                    Section {
                        ForEach(group.tasks, id: \.id) { task in
                            TaskLine(task: task) { editing = form(for: task) }
                                .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                                    Button(role: .destructive) {
                                        store.deleteTask(task.id)
                                    } label: {
                                        Label("Verwijderen", systemImage: "trash")
                                    }
                                }
                        }
                    } header: {
                        Text(group.title).textCase(nil)
                    }
                }
            }
            .listStyle(.insetGrouped)
            .overlay {
                if store.tasks.isEmpty {
                    ContentUnavailableView(
                        "Geen taken", systemImage: "checkmark.circle",
                        description: Text("Tik op + om een taak toe te voegen."))
                }
            }
            .navigationTitle("Taken")
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Menu {
                        Toggle("Voltooide taken tonen", isOn: Binding(get: { store.showDoneTasks }, set: { store.showDoneTasks = $0 }))
                    } label: {
                        Label("Weergave", systemImage: "line.3.horizontal.decrease.circle")
                    }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        editing = newTask()
                    } label: {
                        Label("Nieuwe taak", systemImage: "plus")
                    }
                }
            }
        }
        .sheet(item: Binding(get: { editing.map(EditingTask.init) }, set: { editing = $0?.form })) { item in
            TaskEditor(form: item.form)
        }
    }

    /// The open tasks under "Te laat", "Vandaag", "Morgen", "Later" and "Zonder datum", then
    /// the finished ones.
    private var groups: [TaskGroup] {
        let cal = Calendar.current
        let today = cal.startOfDay(for: .now)
        let tomorrow = cal.date(byAdding: .day, value: 1, to: today) ?? today
        var late: [TaskRow] = [], dueToday: [TaskRow] = [], dueTomorrow: [TaskRow] = [], later: [TaskRow] = []
        var undated: [TaskRow] = [], done: [TaskRow] = []
        for t in store.tasks {
            if t.done {
                done.append(t)
                continue
            }
            guard let due = t.due.map({ Date(timeIntervalSince1970: TimeInterval($0)) }) else {
                undated.append(t)
                continue
            }
            if due < today {
                late.append(t)
            } else if due < tomorrow {
                dueToday.append(t)
            } else if cal.isDate(due, inSameDayAs: tomorrow) {
                dueTomorrow.append(t)
            } else {
                later.append(t)
            }
        }
        return [
            TaskGroup(title: String(localized: "Te laat"), tasks: late),
            TaskGroup(title: String(localized: "Vandaag"), tasks: dueToday),
            TaskGroup(title: String(localized: "Morgen"), tasks: dueTomorrow),
            TaskGroup(title: String(localized: "Later"), tasks: later),
            TaskGroup(title: String(localized: "Zonder datum"), tasks: undated),
            TaskGroup(title: String(localized: "Voltooid"), tasks: done),
        ].filter { !$0.tasks.isEmpty }
    }

    private func form(for t: TaskRow) -> TaskForm {
        TaskForm(
            id: t.id, accountId: t.accountId, title: t.title, notes: t.notes, due: t.due, priority: t.priority,
            repeat: t.repeat, repeatInterval: t.repeatInterval)
    }

    private func newTask() -> TaskForm {
        TaskForm(id: nil, accountId: nil, title: "", notes: "", due: nil, priority: .normal, repeat: nil, repeatInterval: 1)
    }
}

/// A heading of the task list and its tasks.
struct TaskGroup: Identifiable {
    let title: String
    let tasks: [TaskRow]
    var id: String { title }
}

/// The task a sheet edits, with an identity for `sheet(item:)`.
struct EditingTask: Identifiable {
    let form: TaskForm
    var id: String { form.id ?? "new" }
}

struct TaskLine: View {
    @EnvironmentObject private var store: Store
    let task: TaskRow
    let open: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Button {
                store.completeTask(task.id, done: !task.done)
            } label: {
                Image(systemName: task.done ? "checkmark.circle.fill" : "circle")
                    .font(.title2)
                    .foregroundStyle(task.done ? Helder.accent : Helder.dim)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(Text(task.done ? "Weer openen" : "Voltooien"))
            Button(action: open) {
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 4) {
                        if task.priority == .high {
                            Image(systemName: "exclamationmark").foregroundStyle(Helder.flag).fontWeight(.bold)
                                .accessibilityLabel(Text("Hoge prioriteit"))
                        }
                        Text(task.title)
                            .strikethrough(task.done)
                            .foregroundStyle(task.done ? Helder.dim : Helder.text)
                    }
                    let details = detailText
                    if !details.isEmpty {
                        Text(details).font(.footnote).foregroundStyle(overdue ? Helder.flag : Helder.dim)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .padding(.vertical, 2)
    }

    private var overdue: Bool {
        guard !task.done, let due = task.due else { return false }
        return Date(timeIntervalSince1970: TimeInterval(due)) < Calendar.current.startOfDay(for: .now)
    }

    /// "Vr 31 okt · Om de 2 weken · werk@example.org · Laag".
    private var detailText: String {
        var parts: [String] = []
        if let due = task.due {
            parts.append(Date(timeIntervalSince1970: TimeInterval(due)).formatted(.dateTime.weekday(.abbreviated).day().month(.abbreviated)))
        }
        if let repeatText = task.repeatText { parts.append(repeatText) }
        if let address = task.accountAddress { parts.append(address) }
        if task.priority == .low { parts.append(String(localized: "Lage prioriteit")) }
        return parts.joined(separator: " · ")
    }
}

struct TaskEditor: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let original: TaskForm
    @State private var title: String
    @State private var notes: String
    @State private var hasDue: Bool
    @State private var due: Date
    @State private var priority: TaskPriority
    @State private var repeatChoice: Int
    @State private var interval: Int
    @State private var place: String
    @State private var problem: String?
    @State private var confirmDelete = false

    /// The repeat choices; 0 is none.
    private let repeats: [TaskRepeat?] = [nil, .daily, .weekly, .monthly, .yearly]

    init(form: TaskForm) {
        original = form
        _title = State(initialValue: form.title)
        _notes = State(initialValue: form.notes)
        _hasDue = State(initialValue: form.due != nil)
        _due = State(initialValue: form.due.map { Date(timeIntervalSince1970: TimeInterval($0)) } ?? Calendar.current.startOfDay(for: .now))
        _priority = State(initialValue: form.priority)
        _repeatChoice = State(initialValue: [nil, TaskRepeat.daily, .weekly, .monthly, .yearly].firstIndex(of: form.repeat) ?? 0)
        _interval = State(initialValue: Int(max(form.repeatInterval, 1)))
        _place = State(initialValue: form.accountId ?? "")
    }

    /// Only a task on this phone repeats, as on the computer.
    private var canRepeat: Bool { hasDue && place.isEmpty }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Taak", text: $title)
                }
                Section {
                    Toggle("Einddatum", isOn: $hasDue.animation())
                    if hasDue {
                        DatePicker("Datum", selection: $due, displayedComponents: [.date])
                    }
                    Picker("Prioriteit", selection: $priority) {
                        Text("Laag").tag(TaskPriority.low)
                        Text("Normaal").tag(TaskPriority.normal)
                        Text("Hoog").tag(TaskPriority.high)
                    }
                    if canRepeat {
                        Picker("Herhalen", selection: $repeatChoice) {
                            Text("Nooit").tag(0)
                            Text("Elke dag").tag(1)
                            Text("Elke week").tag(2)
                            Text("Elke maand").tag(3)
                            Text("Elk jaar").tag(4)
                        }
                        if repeatChoice > 0 {
                            Stepper("Om de \(interval)", value: $interval, in: 1...99)
                        }
                    }
                }
                if original.id == nil, !store.taskPlaces.isEmpty {
                    Section {
                        Picker("Bewaren in", selection: $place) {
                            Text("Op deze telefoon").tag("")
                            ForEach(store.taskPlaces, id: \.accountId) { p in
                                Text(p.address).tag(p.accountId)
                            }
                        }
                    }
                }
                Section("Notities") {
                    TextField("Notities", text: $notes, axis: .vertical)
                        .lineLimit(3...10)
                }
                if let problem {
                    Section {
                        Text(problem).foregroundStyle(Helder.flag)
                    }
                }
                if original.id != nil {
                    Section {
                        Button("Taak verwijderen", role: .destructive) { confirmDelete = true }
                    }
                }
            }
            .navigationTitle(original.id == nil ? "Nieuwe taak" : "Taak wijzigen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Annuleer") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Bewaar") { save() }
                        .disabled(title.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .confirmationDialog("Deze taak verwijderen?", isPresented: $confirmDelete, titleVisibility: .visible) {
                Button("Verwijderen", role: .destructive) {
                    if let id = original.id { store.deleteTask(id) }
                    dismiss()
                }
            }
        }
    }

    private func save() {
        let form = TaskForm(
            id: original.id,
            accountId: place.isEmpty ? nil : place,
            title: title.trimmingCharacters(in: .whitespaces),
            notes: notes,
            due: hasDue ? Int64(Calendar.current.startOfDay(for: due).timeIntervalSince1970) : nil,
            priority: priority,
            repeat: canRepeat ? repeats[repeatChoice] : nil,
            repeatInterval: UInt32(interval))
        if let message = store.saveTask(form) {
            problem = message
        } else {
            dismiss()
        }
    }
}
