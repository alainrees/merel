import SwiftUI

/// An appointment, or one occurrence of a series, as the calendar opens it.
struct OpenAppointment: Hashable {
    let id: String
    let start: Int64
}

/// The calendar: a week strip, the chosen day on a time line, and an appointment's details on
/// top, as in the design.
struct CalendarTab: View {
    @EnvironmentObject private var store: Store
    @State private var path: [OpenAppointment] = []
    @State private var day = Calendar.current.startOfDay(for: .now)
    @State private var editing: AppointmentForm?

    var body: some View {
        NavigationStack(path: $path) {
            DayScreen(day: $day, editing: $editing)
                .navigationDestination(for: OpenAppointment.self) { open in
                    AppointmentScreen(open: open, editing: $editing, path: $path)
                }
        }
        .sheet(item: Binding(get: { editing.map(EditingForm.init) }, set: { editing = $0?.form })) { item in
            AppointmentEditor(form: item.form)
        }
    }
}

/// The form a sheet edits, with an identity for `sheet(item:)`.
struct EditingForm: Identifiable {
    let form: AppointmentForm
    var id: String { form.id ?? "new" }
}

/// A new appointment on `day` at the next whole hour (or nine o'clock on another day), an hour
/// long.
func newAppointment(on day: Date) -> AppointmentForm {
    let cal = Calendar.current
    let start: Date
    if cal.isDateInToday(day) {
        let next = cal.date(byAdding: .hour, value: 1, to: .now) ?? .now
        start = cal.date(bySetting: .minute, value: 0, of: next) ?? next
    } else {
        start = cal.date(bySettingHour: 9, minute: 0, second: 0, of: day) ?? day
    }
    let t = Int64(start.timeIntervalSince1970)
    return AppointmentForm(
        id: nil, calendarId: nil, title: "", start: t, end: t + 3600, allDay: false, location: "", notes: "",
        reminder: 15)
}

// MARK: - The day

struct DayScreen: View {
    @EnvironmentObject private var store: Store
    @Binding var day: Date
    @Binding var editing: AppointmentForm?

    private var cal: Calendar { Calendar.current }

    var body: some View {
        let next = cal.date(byAdding: .day, value: 1, to: day) ?? day
        let list = store.appointments(from: day, to: next)
        let allDay = list.filter(\.allDay)
        VStack(spacing: 0) {
            WeekStrip(day: $day)
            HStack(alignment: .center, spacing: 8) {
                Text(day.formatted(.dateTime.weekday(.wide).day().month(.wide)).capitalizedFirst)
                    .font(.headline)
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 8)
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(allDay, id: \.self) { a in
                            NavigationLink(value: OpenAppointment(id: a.id, start: a.start)) {
                                AllDayChip(appointment: a)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .fixedSize(horizontal: allDay.count < 3, vertical: false)
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 8)
            TimeLine(day: day, appointments: list.filter { !$0.allDay })
                .card()
                .padding(.horizontal, 12)
                .padding(.bottom, 8)
                .simultaneousGesture(
                    DragGesture(minimumDistance: 40).onEnded { drag in
                        let dx = drag.translation.width
                        guard abs(dx) > abs(drag.translation.height) * 2 else { return }
                        withAnimation { day = cal.date(byAdding: .day, value: dx < 0 ? 1 : -1, to: day) ?? day }
                    })
        }
        .background(Helder.window)
        .navigationTitle(day.formatted(.dateTime.month(.wide)).capitalizedFirst)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    withAnimation { day = cal.startOfDay(for: .now) }
                } label: {
                    Label("Vandaag", systemImage: "calendar.badge.clock")
                }
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    editing = newAppointment(on: day)
                } label: {
                    Label("Nieuwe afspraak", systemImage: "plus")
                }
            }
        }
    }
}

/// The seven days of the chosen day's week; a dot under a day with appointments. Swipe for the
/// week before or after.
struct WeekStrip: View {
    @EnvironmentObject private var store: Store
    @Binding var day: Date

    private var cal: Calendar { Calendar.current }

    var body: some View {
        let start = cal.dateInterval(of: .weekOfYear, for: day)?.start ?? day
        let days = (0..<7).compactMap { cal.date(byAdding: .day, value: $0, to: start) }
        let end = cal.date(byAdding: .day, value: 7, to: start) ?? start
        let busy = Set(store.appointments(from: start, to: end).map { cal.startOfDay(for: Date(timeIntervalSince1970: TimeInterval($0.start))) })
        HStack(spacing: 0) {
            ForEach(days, id: \.self) { d in
                let chosen = cal.isDate(d, inSameDayAs: day)
                Button {
                    withAnimation { day = d }
                } label: {
                    VStack(spacing: 4) {
                        Text(d.formatted(.dateTime.weekday(.abbreviated)))
                            .font(.caption)
                            .foregroundStyle(Helder.dim)
                        Text(d.formatted(.dateTime.day()))
                            .font(.title3)
                            .foregroundStyle(chosen ? Helder.onAccent : (cal.isDateInToday(d) ? Helder.ink : Helder.text))
                            .frame(width: 40, height: 40)
                            .background(chosen ? Helder.accent : .clear, in: Circle())
                        Circle()
                            .fill(busy.contains(cal.startOfDay(for: d)) && !chosen ? Helder.dim : .clear)
                            .frame(width: 5, height: 5)
                    }
                    .frame(maxWidth: .infinity)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(d.formatted(.dateTime.weekday(.wide).day().month(.wide)))
                .accessibilityAddTraits(chosen ? .isSelected : [])
            }
        }
        .padding(.horizontal, 8)
        .padding(.top, 4)
        .simultaneousGesture(
            DragGesture(minimumDistance: 30).onEnded { drag in
                let dx = drag.translation.width
                guard abs(dx) > abs(drag.translation.height) else { return }
                withAnimation { day = cal.date(byAdding: .weekOfYear, value: dx < 0 ? 1 : -1, to: day) ?? day }
            })
    }
}

struct AllDayChip: View {
    let appointment: AppointmentRow

    var body: some View {
        HStack(spacing: 0) {
            Rectangle().fill(Color(rgb: appointment.color)).frame(width: 3)
            Text(appointment.title)
                .font(.subheadline)
                .foregroundStyle(Helder.text)
                .lineLimit(1)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
        }
        .background(Color(rgb: appointment.color).opacity(0.16))
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .accessibilityLabel(Text("\(appointment.title), hele dag"))
    }
}

/// The day's hours with its appointments on them, and a red line at the current time today.
struct TimeLine: View {
    let day: Date
    let appointments: [AppointmentRow]
    @ScaledMetric private var hour: CGFloat = 58
    private let gutter: CGFloat = 56

    var body: some View {
        let dayStart = day.timeIntervalSince1970
        let placed = lanes(appointments)
        ScrollViewReader { reader in
            ScrollView {
                ZStack(alignment: .topLeading) {
                    VStack(spacing: 0) {
                        ForEach(0..<24, id: \.self) { h in
                            HStack(alignment: .top, spacing: 8) {
                                Text(String(format: "%02d:00", h))
                                    .font(.caption)
                                    .monospacedDigit()
                                    .foregroundStyle(Helder.dim)
                                    .frame(width: gutter - 12, alignment: .trailing)
                                    .offset(y: -7)
                                Rectangle().fill(Helder.border).frame(height: 1)
                            }
                            .frame(height: hour, alignment: .top)
                            .id(h)
                            .accessibilityHidden(true)
                        }
                    }
                    .padding(.top, 10)
                    GeometryReader { geo in
                        let width = geo.size.width - gutter - 8
                        ForEach(placed, id: \.row) { p in
                            let a = p.row
                            let top = max(0, CGFloat(Double(a.start) - dayStart) / 3600 * hour)
                            let bottom = min(24 * hour, CGFloat(Double(a.end) - dayStart) / 3600 * hour)
                            let laneWidth = width / CGFloat(p.of)
                            NavigationLink(value: OpenAppointment(id: a.id, start: a.start)) {
                                AppointmentBlock(appointment: a, height: max(26, bottom - top - 2))
                            }
                            .buttonStyle(.plain)
                            .frame(width: laneWidth - 3, height: max(26, bottom - top - 2))
                            .offset(x: gutter + CGFloat(p.lane) * laneWidth, y: top + 10 + 1)
                        }
                        if Calendar.current.isDateInToday(day) {
                            let y = CGFloat(Date.now.timeIntervalSince1970 - dayStart) / 3600 * hour + 10
                            HStack(spacing: 0) {
                                Circle().fill(Helder.flag).frame(width: 9, height: 9)
                                Rectangle().fill(Helder.flag).frame(height: 2)
                            }
                            .offset(x: gutter - 6, y: y - 4.5)
                            .accessibilityHidden(true)
                        }
                    }
                    .frame(height: 24 * hour + 10)
                }
                .padding(.bottom, 12)
            }
            .onAppear { scroll(reader) }
            .onChange(of: day) { _, _ in scroll(reader) }
        }
    }

    /// The first appointment's hour, else the current hour today, else eight o'clock.
    private func scroll(_ reader: ScrollViewProxy) {
        let cal = Calendar.current
        var h = 8
        if let first = appointments.first {
            h = cal.component(.hour, from: Date(timeIntervalSince1970: TimeInterval(first.start)))
        } else if cal.isDateInToday(day) {
            h = cal.component(.hour, from: .now)
        }
        reader.scrollTo(max(0, min(h, 23) - 1), anchor: .top)
    }
}

/// An appointment with its place among the ones it overlaps: lane `lane` of `of`.
struct Placed {
    let row: AppointmentRow
    let lane: Int
    let of: Int
}

/// Puts overlapping appointments side by side, as the desktop's day view does.
func lanes(_ list: [AppointmentRow]) -> [Placed] {
    var out: [Placed] = []
    var group: [(AppointmentRow, Int)] = []
    var groupEnd: Int64 = .min
    func close() {
        let count = (group.map(\.1).max() ?? 0) + 1
        out += group.map { Placed(row: $0.0, lane: $0.1, of: count) }
        group = []
    }
    for a in list.sorted(by: { ($0.start, $0.end) < ($1.start, $1.end) }) {
        if a.start >= groupEnd { close() }
        var lane = 0
        while group.contains(where: { $0.1 == lane && $0.0.end > a.start }) { lane += 1 }
        group.append((a, lane))
        groupEnd = max(groupEnd, a.end)
    }
    close()
    return out
}

struct AppointmentBlock: View {
    @Environment(\.openURL) private var openURL
    let appointment: AppointmentRow
    let height: CGFloat

    var body: some View {
        let a = appointment
        HStack(spacing: 0) {
            Rectangle().fill(Color(rgb: a.color)).frame(width: 4)
            HStack(alignment: .top, spacing: 6) {
                VStack(alignment: .leading, spacing: 1) {
                    if height < 44 {
                        (Text(a.title).fontWeight(.semibold) + Text(" · " + timeRange(a)).foregroundColor(Helder.dim))
                            .font(.subheadline)
                            .lineLimit(1)
                    } else {
                        Text(a.title).font(.subheadline.weight(.semibold)).lineLimit(2)
                        Text([timeRange(a), place(a)].filter { !$0.isEmpty }.joined(separator: " · "))
                            .font(.caption)
                            .foregroundStyle(Helder.dim)
                            .lineLimit(2)
                    }
                }
                .foregroundStyle(Helder.text)
                Spacer(minLength: 0)
                if let link = a.onlineUrl.flatMap(URL.init(string:)), height >= 44 {
                    Button {
                        openURL(link)
                    } label: {
                        Label("Deelnemen", systemImage: "video")
                            .font(.caption.weight(.semibold))
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                            .foregroundStyle(Helder.onAccent)
                            .background(Helder.accent, in: RoundedRectangle(cornerRadius: 6))
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .background(Color(rgb: a.color).opacity(0.16))
        .background(Helder.card)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .accessibilityElement(children: .combine)
    }

    private func place(_ a: AppointmentRow) -> String {
        if !a.location.isEmpty { return a.location }
        return a.onlineUrl == nil ? "" : String(localized: "Onlinevergadering")
    }
}

/// "14:00–15:00", on the phone's clock.
func timeRange(_ a: AppointmentRow) -> String {
    let from = Date(timeIntervalSince1970: TimeInterval(a.start))
    let to = Date(timeIntervalSince1970: TimeInterval(a.end))
    return from.formatted(date: .omitted, time: .shortened) + "–" + to.formatted(date: .omitted, time: .shortened)
}

/// "Donderdag 23 oktober · 14:00–15:00", or for a whole day "Donderdag 23 oktober · Hele dag".
func whenText(_ a: AppointmentRow) -> String {
    let from = Date(timeIntervalSince1970: TimeInterval(a.start))
    let date = from.formatted(.dateTime.weekday(.wide).day().month(.wide)).capitalizedFirst
    if a.allDay {
        let days = Int((a.end - a.start + 3600) / 86_400)
        if days > 1 {
            let last = Date(timeIntervalSince1970: TimeInterval(a.end - 1))
            return date + " – " + last.formatted(.dateTime.weekday(.wide).day().month(.wide))
        }
        return date + " · " + String(localized: "Hele dag")
    }
    return date + " · " + timeRange(a)
}

extension String {
    /// "donderdag 23 oktober" as a heading: "Donderdag 23 oktober".
    var capitalizedFirst: String { prefix(1).uppercased() + dropFirst() }
}

// MARK: - One appointment

struct AppointmentScreen: View {
    @EnvironmentObject private var store: Store
    @Environment(\.openURL) private var openURL
    let open: OpenAppointment
    @Binding var editing: AppointmentForm?
    @Binding var path: [OpenAppointment]
    @State private var confirmingDelete = false

    var body: some View {
        let day = Date(timeIntervalSince1970: TimeInterval(open.start))
        let start = Calendar.current.startOfDay(for: day)
        let end = Calendar.current.date(byAdding: .day, value: 1, to: start) ?? start
        // Look a little wider for an appointment that began on an earlier day.
        let found = store.appointments(from: start.addingTimeInterval(-40 * 86_400), to: end)
            .first { $0.id == open.id && $0.start == open.start }
        Group {
            if let a = found {
                details(a)
            } else {
                ContentUnavailableView("Deze afspraak is er niet meer", systemImage: "calendar.badge.exclamationmark")
            }
        }
        .background(Helder.window)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let a = found {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(role: .destructive) {
                        confirmingDelete = true
                    } label: {
                        Label("Verwijderen", systemImage: "trash")
                    }
                }
                if a.recurrence == nil {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Wijzig") { editing = form(for: a) }
                    }
                }
            }
        }
        .confirmationDialog("Afspraak verwijderen?", isPresented: $confirmingDelete, titleVisibility: .visible) {
            if let a = found {
                if a.recurrence != nil {
                    Button("Alleen deze keer", role: .destructive) { remove(a, onlyThis: true) }
                    Button("De hele reeks", role: .destructive) { remove(a, onlyThis: false) }
                } else {
                    Button("Verwijderen", role: .destructive) { remove(a, onlyThis: false) }
                }
            }
        }
    }

    private func remove(_ a: AppointmentRow, onlyThis: Bool) {
        store.deleteAppointment(a, onlyThis: onlyThis)
        path.removeAll()
    }

    private func form(for a: AppointmentRow) -> AppointmentForm {
        AppointmentForm(
            id: a.id, calendarId: a.calendarId, title: a.title, start: a.start, end: a.end, allDay: a.allDay,
            location: a.location, notes: a.notes, reminder: a.reminder)
    }

    @ViewBuilder
    private func details(_ a: AppointmentRow) -> some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack(alignment: .top, spacing: 10) {
                    RoundedRectangle(cornerRadius: 2).fill(Color(rgb: a.color)).frame(width: 5)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(a.title)
                            .font(.title2.weight(.bold))
                            .foregroundStyle(Helder.text)
                            .accessibilityAddTraits(.isHeader)
                        Text(whenText(a))
                            .font(.body)
                            .foregroundStyle(Helder.dim)
                    }
                }
                .fixedSize(horizontal: false, vertical: true)

                if let link = a.onlineUrl.flatMap(URL.init(string:)) {
                    HStack(spacing: 12) {
                        Image(systemName: "video")
                            .foregroundStyle(Helder.ink)
                            .frame(width: 44, height: 44)
                            .background(Helder.selected, in: RoundedRectangle(cornerRadius: 10))
                            .accessibilityHidden(true)
                        VStack(alignment: .leading, spacing: 2) {
                            Text("Onlinevergadering").font(.headline).foregroundStyle(Helder.text)
                            Text(link.host() ?? "").font(.subheadline).foregroundStyle(Helder.dim)
                        }
                        Spacer(minLength: 8)
                        Button("Deelnemen") { openURL(link) }
                            .buttonStyle(.borderedProminent)
                            .tint(Helder.accent)
                            .foregroundStyle(Helder.onAccent)
                    }
                    .padding(14)
                    .card()
                }

                VStack(spacing: 0) {
                    let lines = infoLines(a)
                    ForEach(Array(lines.enumerated()), id: \.offset) { i, line in
                        if i > 0 { Divider().padding(.leading, 52) }
                        HStack(alignment: .top, spacing: 14) {
                            Image(systemName: line.icon)
                                .foregroundStyle(Helder.dim)
                                .frame(width: 24)
                                .accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(line.text).foregroundStyle(Helder.text)
                                if let detail = line.detail {
                                    Text(detail).font(.subheadline).foregroundStyle(Helder.dim)
                                }
                            }
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 14)
                        .padding(.vertical, 12)
                        .accessibilityElement(children: .combine)
                    }
                }
                .card()

                if !a.attendees.isEmpty {
                    Text("Deelnemers · \(a.attendees.count)")
                        .font(.subheadline)
                        .foregroundStyle(Helder.dim)
                        .accessibilityAddTraits(.isHeader)
                    VStack(spacing: 0) {
                        ForEach(a.attendees, id: \.email) { p in
                            AttendeeLine(person: p)
                        }
                    }
                    .padding(.vertical, 4)
                    .card()
                }

                if !a.notes.isEmpty {
                    Text(a.notes)
                        .foregroundStyle(Helder.text)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(14)
                        .card()
                }
            }
            .padding(16)
        }
    }

    private struct Line {
        let icon: String
        let text: String
        var detail: String?
    }

    private func infoLines(_ a: AppointmentRow) -> [Line] {
        var lines: [Line] = []
        if !a.location.isEmpty { lines.append(Line(icon: "mappin.and.ellipse", text: a.location)) }
        lines.append(Line(icon: "bell", text: reminderText(a.reminder)))
        if let c = store.calendar(a.calendarId) {
            lines.append(Line(icon: "calendar", text: c.name, detail: c.address))
        }
        if let r = a.recurrence { lines.append(Line(icon: "repeat", text: r)) }
        return lines
    }
}

/// "15 minuten van tevoren", "Geen herinnering".
func reminderText(_ minutes: UInt32?) -> String {
    guard let m = minutes else { return String(localized: "Geen herinnering") }
    switch m {
    case 0: return String(localized: "Bij het begin")
    case 1440: return String(localized: "1 dag van tevoren")
    case let m where m % 60 == 0: return String(localized: "\(Int(m / 60)) uur van tevoren")
    default: return String(localized: "\(Int(m)) minuten van tevoren")
    }
}

struct AttendeeLine: View {
    let person: AttendeeRow

    var body: some View {
        HStack(spacing: 12) {
            Avatar(name: person.name, address: person.email, size: 40)
            VStack(alignment: .leading, spacing: 1) {
                Text(person.me ? String(localized: "\(person.name) (jij)") : person.name)
                    .foregroundStyle(Helder.text)
                Text(status).font(.subheadline).foregroundStyle(Helder.dim)
            }
            Spacer(minLength: 8)
            Image(systemName: icon)
                .foregroundStyle(Helder.dim)
                .accessibilityHidden(true)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 6)
        .accessibilityElement(children: .combine)
    }

    private var status: String {
        if person.organizer { return String(localized: "Organisator") }
        let answer: String
        switch person.answer {
        case .accepted: answer = String(localized: "Geaccepteerd")
        case .tentative: answer = String(localized: "Voorlopig")
        case .declined: answer = String(localized: "Geweigerd")
        case .none: answer = String(localized: "Nog geen antwoord")
        }
        return person.optional ? answer + " · " + String(localized: "optioneel") : answer
    }

    private var icon: String {
        if person.organizer { return "checkmark.circle" }
        switch person.answer {
        case .accepted: return "checkmark.circle"
        case .tentative: return "questionmark.circle"
        case .declined: return "xmark.circle"
        case .none: return "clock"
        }
    }
}

// MARK: - New and changed appointments

struct AppointmentEditor: View {
    @EnvironmentObject private var store: Store
    @Environment(\.dismiss) private var dismiss
    let original: AppointmentForm
    @State private var title: String
    @State private var location: String
    @State private var allDay: Bool
    @State private var start: Date
    @State private var end: Date
    @State private var calendarId: String
    @State private var reminder: Int
    @State private var notes: String
    @State private var problem: String?

    /// The reminder choices, in minutes; -1 for none.
    private let reminders = [-1, 0, 5, 15, 30, 60, 120, 1440]

    init(form: AppointmentForm) {
        original = form
        _title = State(initialValue: form.title)
        _location = State(initialValue: form.location)
        _allDay = State(initialValue: form.allDay)
        let s = Date(timeIntervalSince1970: TimeInterval(form.start))
        // An all-day appointment's end is the next midnight; the form shows its last day.
        let e = Date(timeIntervalSince1970: TimeInterval(form.allDay ? form.end - 1 : form.end))
        _start = State(initialValue: s)
        _end = State(initialValue: e)
        _calendarId = State(initialValue: form.calendarId ?? "")
        _reminder = State(initialValue: form.reminder.map(Int.init) ?? -1)
        _notes = State(initialValue: form.notes)
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Onderwerp", text: $title)
                    TextField("Locatie", text: $location)
                }
                Section {
                    Toggle("Hele dag", isOn: $allDay)
                    DatePicker("Begin", selection: $start, displayedComponents: allDay ? [.date] : [.date, .hourAndMinute])
                    DatePicker("Einde", selection: $end, in: start..., displayedComponents: allDay ? [.date] : [.date, .hourAndMinute])
                }
                Section {
                    if store.calendars.count > 1 {
                        Picker("Agenda", selection: $calendarId) {
                            ForEach(store.calendars, id: \.id) { c in
                                Text(c.address.map { "\(c.name) (\($0))" } ?? c.name).tag(c.id)
                            }
                        }
                    }
                    Picker("Herinnering", selection: $reminder) {
                        ForEach(reminders, id: \.self) { m in
                            Text(reminderText(m < 0 ? nil : UInt32(m))).tag(m)
                        }
                    }
                }
                Section("Notities") {
                    TextField("Notities", text: $notes, axis: .vertical)
                        .lineLimit(3...10)
                }
            }
            .navigationTitle(original.id == nil ? "Nieuwe afspraak" : "Afspraak wijzigen")
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
            .onChange(of: start) { old, new in
                // Moving the start takes the end along, as Outlook does.
                end = end.addingTimeInterval(new.timeIntervalSince(old))
            }
            .onAppear {
                if calendarId.isEmpty { calendarId = store.calendars.first?.id ?? "" }
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
    }

    private func save() {
        let cal = Calendar.current
        var from = start
        var to = end
        if allDay {
            from = cal.startOfDay(for: start)
            to = cal.date(byAdding: .day, value: 1, to: cal.startOfDay(for: end)) ?? from
        }
        let form = AppointmentForm(
            id: original.id, calendarId: calendarId.isEmpty ? nil : calendarId, title: title,
            start: Int64(from.timeIntervalSince1970), end: Int64(to.timeIntervalSince1970), allDay: allDay,
            location: location, notes: notes, reminder: reminder < 0 ? nil : UInt32(reminder))
        if let message = store.saveAppointment(form) {
            problem = message
        } else {
            dismiss()
        }
    }
}
