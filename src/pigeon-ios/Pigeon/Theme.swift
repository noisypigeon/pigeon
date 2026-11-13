// The look the mockup settled: a dot grid, block shadows, SF Pro and SF Mono, light and dark;
// see .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md §1.

import SwiftUI
import UIKit

enum Theme {
    static func color(_ light: UInt32, _ dark: UInt32) -> Color {
        Color(
            UIColor { traits in
                let v = traits.userInterfaceStyle == .dark ? dark : light
                return UIColor(
                    red: CGFloat((v >> 16) & 0xff) / 255,
                    green: CGFloat((v >> 8) & 0xff) / 255,
                    blue: CGFloat(v & 0xff) / 255,
                    alpha: 1)
            })
    }

    static let background = color(0xF4EEE3, 0x14121A)
    static let dot = color(0xCFC4B2, 0x2F2B38)
    static let card = color(0xFFFDF8, 0x211E26)
    static let ink = color(0x1B1A17, 0xF2ECDF)
    static let ink2 = color(0x45403A, 0xD3CCBE)
    static let muted = color(0x6A6259, 0xA79F93)
    static let chip = color(0xEAE1D2, 0x2C2833)
    static let line = color(0xCBBFAE, 0x4A4553)
    static let shadow = color(0x1B1A17, 0x000000)
    static let accent = color(0x7A2E4F, 0xE07AA6)
    static let onAccent = color(0xFFFDF8, 0x14121A)
    static let accentSoft = color(0xF2D3DE, 0x3D2432)
    static let good = color(0x2F6B4F, 0x5FBF8E)

    /// The sources' colours, checked for colour-blind separation on both surfaces.
    static func source(_ name: String) -> Color {
        switch name {
        case "cloudflare": return color(0xB04A78, 0xC65A8C)
        case "github": return color(0xB8862B, 0xB88A2E)
        default: return color(0x3A78B5, 0x4C86C6)
        }
    }

    /// The kinds' colours, in a source's order, distinct from the sources'.
    static func kind(_ index: Int) -> Color {
        switch index % 3 {
        case 0: return color(0x6A55C9, 0x8471DC)
        case 1: return color(0xC2571F, 0xD06A35)
        default: return color(0x2D8C6A, 0x2F9C78)
        }
    }

    static func display(_ size: CGFloat) -> Font { .system(size: size, weight: .bold) }
    static func mono(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        .system(size: size, weight: weight, design: .monospaced)
    }
}

/// The dot grid behind every screen.
struct DotGrid: View {
    var body: some View {
        Canvas { context, size in
            let step: CGFloat = 16
            var y: CGFloat = 8
            while y < size.height {
                var x: CGFloat = 8
                while x < size.width {
                    context.fill(
                        Path(ellipseIn: CGRect(x: x - 1.1, y: y - 1.1, width: 2.2, height: 2.2)),
                        with: .color(Theme.dot))
                    x += step
                }
                y += step
            }
        }
        .background(Theme.background)
        .ignoresSafeArea()
    }
}

/// A card: a surface with a hard offset shadow and no outline.
struct Block: ViewModifier {
    var radius: CGFloat = 14
    var shadow: Color = Theme.shadow

    func body(content: Content) -> some View {
        content
            .background(RoundedRectangle(cornerRadius: radius).fill(Theme.card))
            .background(
                RoundedRectangle(cornerRadius: radius)
                    .fill(shadow)
                    .offset(x: 3, y: 3)
            )
    }
}

extension View {
    func block(radius: CGFloat = 14, shadow: Color = Theme.shadow) -> some View {
        modifier(Block(radius: radius, shadow: shadow))
    }
}

/// A label in SF Mono capitals.
struct Eyebrow: View {
    let text: String
    var body: some View {
        Text(text.uppercased())
            .font(Theme.mono(12, .semibold))
            .tracking(1)
            .foregroundStyle(Theme.muted)
    }
}
