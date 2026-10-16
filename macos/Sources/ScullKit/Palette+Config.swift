// The configured colours laid over the built-in palette.

import AppKit

extension Palette {
    public init(_ settings: ScullSettings) {
        self.init()
        foreground = settings.foreground
        background = settings.background
        cursor = settings.cursor
        // The config has no selection colour yet; the system accent, toned
        // down over the background, keeps the text on it readable.
        if let accent = NSColor.controlAccentColor.usingColorSpace(.sRGB) {
            let rgb = [accent.redComponent, accent.greenComponent, accent.blueComponent]
                .reduce(UInt32(0)) { $0 << 8 | UInt32(($1 * 255).rounded()) }
            selection = Self.blend(rgb, over: background, alpha: 0.45)
        }
        // The core sends the sixteen it resolved; the 6x6x6 cube and the greys stay xterm's.
        for (i, color) in settings.ansi.prefix(16).enumerated() { indexed[i] = color }
    }
}
