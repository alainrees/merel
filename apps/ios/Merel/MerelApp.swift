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

/// The five modules along the bottom, as in the design.
struct RootView: View {
    @EnvironmentObject private var store: Store

    var body: some View {
        TabView {
            MailTab()
                .tabItem { Label("Mail", systemImage: "envelope") }
            CalendarTab()
                .tabItem { Label("Agenda", systemImage: "calendar") }
            ChatsTab()
                .tabItem { Label("Chat", systemImage: "bubble.left.and.bubble.right") }
            ContactsTab()
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
