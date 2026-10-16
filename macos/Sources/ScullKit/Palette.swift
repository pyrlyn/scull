// Colours as the core hands them out (default, palette index or RGB)
// resolved to sRGB. The palette is xterm's, so programs that pick indexed
// colours look as they do elsewhere until themes arrive.

import CoreGraphics
import CScull

public struct Palette: Sendable {
    public var foreground: UInt32 = 0xE5E5E5
    public var background: UInt32 = 0x141414
    public var cursor: UInt32 = 0xE5E5E5
    /// Backgrounds of selected cells, search matches and the current match.
    public var selection: UInt32 = 0x264F78
    public var match: UInt32 = 0x6B5A1A
    public var currentMatch: UInt32 = 0xF2A33A
    /// Text on the current match, whose background is bright in any theme.
    public var currentMatchText: UInt32 = 0x000000
    public internal(set) var indexed: [UInt32]

    public init() {
        let ansi: [UInt32] = [
            0x000000, 0xCD0000, 0x00CD00, 0xCDCD00, 0x0000EE, 0xCD00CD, 0x00CDCD, 0xE5E5E5,
            0x7F7F7F, 0xFF0000, 0x00FF00, 0xFFFF00, 0x5C5CFF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
        ]
        // xterm's cube levels (256colres.pl), which most terminals copy, so
        // indexed colours match what programs were tuned against.
        let levels: [UInt32] = [0, 95, 135, 175, 215, 255]
        var cube: [UInt32] = []
        for r in levels {
            for g in levels {
                for b in levels { cube.append(r << 16 | g << 8 | b) }
            }
        }
        let greys = (0..<24).map { (i: UInt32) -> UInt32 in
            let level = 8 + 10 * i
            return level << 16 | level << 8 | level
        }
        indexed = ansi + cube + greys
    }

    /// The RGB value of a core colour, with `fallback` for the default.
    public func rgb(_ color: UInt32, fallback: UInt32) -> UInt32 {
        let kind = color & UInt32(TT_COLOR_KIND_MASK)
        if kind == UInt32(TT_COLOR_INDEXED) {
            return indexed[Int(color & 0xFF)]
        }
        if kind == UInt32(TT_COLOR_RGB) {
            return color & 0xFF_FFFF
        }
        return fallback
    }

    /// Foreground and background of a style, reverse video applied.
    public func colors(of style: tt_style) -> (fg: UInt32, bg: UInt32) {
        let fg = rgb(style.fg, fallback: foreground)
        let bg = rgb(style.bg, fallback: background)
        return style.attrs & UInt16(TT_ATTR_INVERSE) != 0 ? (bg, fg) : (fg, bg)
    }

    /// The background a cell shows: the current match over the selection
    /// over other matches over its own `bg`.
    public func background(flags: UInt8, bg: UInt32) -> UInt32 {
        let flags = Int32(flags)
        if flags & TT_CELL_CURRENT_MATCH != 0 { return currentMatch }
        if flags & TT_CELL_SELECTED != 0 { return selection }
        return flags & TT_CELL_MATCH != 0 ? match : bg
    }

    /// `top` laid over `bottom` with opacity `alpha`.
    public static func blend(_ top: UInt32, over bottom: UInt32, alpha: Double) -> UInt32 {
        [16, 8, 0].reduce(0) { rgb, shift in
            let (t, b) = (Double(top >> shift & 0xFF), Double(bottom >> shift & 0xFF))
            return rgb | UInt32((t * alpha + b * (1 - alpha)).rounded()) << shift
        }
    }

    public static func cgColor(_ rgb: UInt32, alpha: CGFloat = 1) -> CGColor {
        let channel = { (shift: UInt32) in CGFloat((rgb >> shift) & 0xFF) / 255 }
        return CGColor(srgbRed: channel(16), green: channel(8), blue: channel(0), alpha: alpha)
    }
}
