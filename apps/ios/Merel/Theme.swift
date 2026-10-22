import SwiftUI
import UIKit

/// Merel's "Helder" colours, the same tokens as the desktop's `theme.rs`, light and dark.
/// Every text colour meets WCAG 2.2 AA (4.5:1) on the background it is drawn on.
enum Helder {
    static let window = dynamic(light: 0xF3F4F6, dark: 0x1B1C1E)
    static let card = dynamic(light: 0xFFFFFF, dark: 0x26282B)
    static let border = dynamic(light: 0xE3E5E8, dark: 0x3A3D42)
    static let text = dynamic(light: 0x242424, dark: 0xEEEEEE)
    static let dim = dynamic(light: 0x616161, dark: 0xC8C8C8)
    static let accent = dynamic(light: 0x1F6FC9, dark: 0x6FA8EE)
    /// Accent-coloured text: links, unread subjects, toolbar buttons.
    static let ink = dynamic(light: 0x1757A3, dark: 0x95C0F4)
    /// Text on an accent-coloured button.
    static let onAccent = dynamic(light: 0xFFFFFF, dark: 0x0B1A2E)
    static let selected = dynamic(light: 0xDCE8F7, dark: 0x1E3552)
    static let flag = dynamic(light: 0xC4314B, dark: 0xF07A8C)

    /// The avatar colour for an address: always the same for the same person, as on the desktop.
    static func avatar(for address: String) -> Color {
        // helder::AVATAR_COLORS and its hash; white initials are at least 4.5:1 on each.
        let palette: [UInt32] = [0x1F5FA8, 0x2E6E50, 0x6B4FA8, 0x8A5300, 0xA11F35, 0x0F666E, 0x4A4E54, 0x8C2F86]
        var hash: UInt32 = 0
        for byte in address.lowercased().utf8 {
            hash = hash &* 31 &+ UInt32(byte)
        }
        return Color(rgb: palette[Int(hash % UInt32(palette.count))])
    }

    private static func dynamic(light: UInt32, dark: UInt32) -> Color {
        Color(UIColor { $0.userInterfaceStyle == .dark ? UIColor(rgb: dark) : UIColor(rgb: light) })
    }
}

extension UIColor {
    convenience init(rgb: UInt32) {
        self.init(
            red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
    }
}

extension Color {
    init(rgb: UInt32) {
        self.init(UIColor(rgb: rgb))
    }
}

/// The initials Merel shows in an avatar: first letters of the first and last word, skipping
/// anything in brackets, as the desktop does.
func initials(_ name: String) -> String {
    let words = (name.split(separator: "(", maxSplits: 1).first.map(String.init) ?? name)
        .split(whereSeparator: { $0.isWhitespace }).filter { $0.first?.isLetter == true || $0.first?.isNumber == true }
    guard let first = words.first?.first else { return "" }
    if words.count > 1, let last = words.last?.first {
        return String(first).uppercased() + String(last).uppercased()
    }
    return String(first).uppercased()
}

struct Avatar: View {
    let name: String
    let address: String
    var size: CGFloat = 40

    var body: some View {
        Text(initials(name).isEmpty ? initials(address) : initials(name))
            .font(.system(size: size * 0.38, weight: .regular))
            .foregroundStyle(.white)
            .frame(width: size, height: size)
            .background(Helder.avatar(for: address), in: Circle())
            .accessibilityHidden(true)
    }
}

/// A white card with Helder's rounded border.
struct CardStyle: ViewModifier {
    func body(content: Content) -> some View {
        content
            .background(Helder.card, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(Helder.border, lineWidth: 1))
    }
}

extension View {
    func card() -> some View { modifier(CardStyle()) }
}
