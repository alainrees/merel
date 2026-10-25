import SafariServices
import UIKit

/// Opens a sign-in link in Safari inside the app, over whatever is on screen. Microsoft or
/// Google send the person back to `http://127.0.0.1`, where Merel's engine listens for this
/// one sign-in, so the app never sees the password; the store closes Safari when it is done.
final class SafariSignIn: SignInBrowser, @unchecked Sendable {
    static let shared = SafariSignIn()
    @MainActor private weak var shown: SFSafariViewController?

    func show(url: String) -> Bool {
        guard let link = URL(string: url), link.scheme == "https" else { return false }
        Task { @MainActor in
            guard let top = Self.topController() else { return }
            let safari = SFSafariViewController(url: link)
            safari.dismissButtonStyle = .cancel
            shown = safari
            top.present(safari, animated: true)
        }
        return true
    }

    @MainActor func close() {
        shown?.dismiss(animated: true)
        shown = nil
    }

    @MainActor private static func topController() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        var top = scenes.flatMap(\.windows).first { $0.isKeyWindow }?.rootViewController
        while let presented = top?.presentedViewController { top = presented }
        return top
    }
}
