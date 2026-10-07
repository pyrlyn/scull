// The configured colours laid over the built-in palette.

extension Palette {
    public init(_ settings: ScullSettings) {
        self.init()
        foreground = settings.foreground
        background = settings.background
        cursor = settings.cursor
        // The core sends the sixteen it resolved; the 6x6x6 cube and the greys stay xterm's.
        for (i, color) in settings.ansi.prefix(16).enumerated() { indexed[i] = color }
    }
}
