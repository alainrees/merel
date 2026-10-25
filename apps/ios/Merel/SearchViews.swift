import SwiftUI

/// Search settings: search on meaning, a small language model on the phone itself.
struct SearchSettings: View {
    @EnvironmentObject private var store: Store

    var body: some View {
        Form {
            Section {
                Toggle(
                    "Zoeken op betekenis",
                    isOn: Binding(get: { store.searchOnMeaning }, set: { store.setSearchOnMeaning($0) }))
                if store.searchOnMeaning, let status = store.meaningStatus {
                    MeaningStatusView(status: status)
                }
            } footer: {
                Text(
                    "Merel vindt dan ook mail over hetzelfde als er andere woorden in staan, zoals een offerte als je naar een voorstel zoekt. Een klein taalmodel doet dat op je telefoon zelf; je mail gaat nergens heen. Aanzetten haalt het model eenmalig op (\(Int(meaningDownloadMegabytes())) MB), gebruik daarvoor bij voorkeur wifi."
                )
            }
        }
        .navigationTitle("Zoeken")
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct MeaningStatusView: View {
    @EnvironmentObject private var store: Store
    let status: MeaningStatus

    var body: some View {
        switch status.state {
        case .downloading(let done, let total):
            VStack(alignment: .leading, spacing: 6) {
                Text("Taalmodel ophalen…")
                ProgressView(value: Double(done), total: Double(max(total, 1)))
                Text("\(bytes(done)) van \(bytes(total))").font(.footnote).foregroundStyle(Helder.dim)
            }
            .accessibilityElement(children: .combine)
        case .preparing:
            HStack {
                Text("Taalmodel klaarmaken…")
                Spacer()
                ProgressView()
            }
        case .ready:
            VStack(alignment: .leading, spacing: 4) {
                Text(status.busy ? "Berichten lezen…" : "Klaar")
                Text("\(Int(status.messages)) berichten gelezen · model \(bytes(status.modelBytes))")
                    .font(.footnote).foregroundStyle(Helder.dim)
            }
            .accessibilityElement(children: .combine)
        case .failed(let reason):
            VStack(alignment: .leading, spacing: 6) {
                Label("Ophalen is niet gelukt", systemImage: "exclamationmark.triangle")
                    .foregroundStyle(Helder.flag)
                Text(reason).font(.footnote).foregroundStyle(Helder.dim)
                Button("Opnieuw proberen") { store.retryMeaning() }
            }
        }
    }

    private func bytes(_ n: UInt64) -> String {
        Int64(clamping: n).formatted(.byteCount(style: .file))
    }
}
