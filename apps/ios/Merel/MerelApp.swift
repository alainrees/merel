import SwiftUI

@main
struct MerelApp: App {
    @StateObject private var store = Store.live()
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(store)
                .tint(Helder.ink)
        }
        .onChange(of: phase) { _, now in
            if now == .background { store.flush() }
        }
    }
}

/// The five modules along the bottom, as in the design; mail, the calendar and tasks are built.
struct RootView: View {
    @EnvironmentObject private var store: Store

    var body: some View {
        TabView {
            MailTab()
                .tabItem { Label("Mail", systemImage: "envelope") }
            CalendarTab()
                .tabItem { Label("Agenda", systemImage: "calendar") }
            Soon(title: "Chat")
                .tabItem { Label("Chat", systemImage: "bubble.left.and.bubble.right") }
            Soon(title: "Contacten")
                .tabItem { Label("Contacten", systemImage: "person.2") }
            TasksTab()
                .tabItem { Label("Taken", systemImage: "checkmark.square") }
        }
        .alert(
            "Dat lukte niet",
            isPresented: Binding(get: { store.problem != nil }, set: { if !$0 { store.problem = nil } })
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(store.problem ?? "")
        }
    }
}

/// A module that is not on the phone yet.
struct Soon: View {
    let title: LocalizedStringKey

    var body: some View {
        NavigationStack {
            ContentUnavailableView(
                title, systemImage: "hammer",
                description: Text("Dit onderdeel komt binnenkort naar de telefoon."))
                .background(Helder.window)
                .navigationTitle(title)
        }
    }
}
