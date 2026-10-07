import AppKit
import CScull
import Testing
@testable import ScullKit

@Test func paletteResolvesDefaultIndexedAndRgbColours() {
    let palette = Palette()
    #expect(palette.indexed.count == 256)
    #expect(palette.rgb(UInt32(TT_COLOR_DEFAULT), fallback: 0x123456) == 0x123456)
    #expect(palette.rgb(UInt32(TT_COLOR_INDEXED) | 1, fallback: 0) == 0xCD0000)
    #expect(palette.rgb(UInt32(TT_COLOR_INDEXED) | 16, fallback: 1) == 0x000000)
    #expect(palette.rgb(UInt32(TT_COLOR_INDEXED) | 231, fallback: 0) == 0xFFFFFF)
    #expect(palette.rgb(UInt32(TT_COLOR_INDEXED) | 232, fallback: 0) == 0x080808)
    #expect(palette.rgb(UInt32(TT_COLOR_RGB) | 0xABCDEF, fallback: 0) == 0xABCDEF)
}

@Test func inverseSwapsResolvedColours() {
    let palette = Palette()
    var style = tt_style()
    style.fg = UInt32(TT_COLOR_INDEXED) | 2
    style.attrs = UInt16(TT_ATTR_INVERSE)
    let colors = palette.colors(of: style)
    #expect(colors.fg == palette.background)
    #expect(colors.bg == 0x00CD00)
}

@Test func keyCodesMapByPosition() {
    #expect(KeyMap.key(forKeyCode: 0x00) == UInt32(("a" as Unicode.Scalar).value))
    #expect(KeyMap.key(forKeyCode: 0x2A) == UInt32(("\\" as Unicode.Scalar).value))
    #expect(KeyMap.key(forKeyCode: 0x32) == UInt32(("`" as Unicode.Scalar).value))
    #expect(KeyMap.key(forKeyCode: 0x35) == UInt32(TT_KEY_ESCAPE))
    #expect(KeyMap.key(forKeyCode: 0x77) == UInt32(TT_KEY_END))
    #expect(KeyMap.key(forKeyCode: 0x6F) == UInt32(TT_KEY_F1) + 11)
    #expect(KeyMap.key(forKeyCode: 0x5C) == UInt32(TT_KEY_KP_0) + 9)
    #expect(KeyMap.key(forKeyCode: 0x51) == UInt32(TT_KEY_KP_0) + 16)
    #expect(KeyMap.key(forKeyCode: 0x0A) == nil)
}

@Test func modifierFlagsMapToCoreBits() {
    #expect(KeyMap.mods([]) == 0)
    let all = KeyMap.mods([.shift, .option, .control, .command, .capsLock])
    #expect(all == UInt8(TT_MOD_SHIFT | TT_MOD_ALT | TT_MOD_CTRL | TT_MOD_SUPER | TT_MOD_CAPS_LOCK))
}

@MainActor
@Test func sessionShowsFedOutputInItsFrame() throws {
    let session = try TerminalSession(cols: 10, rows: 3)
    #expect(session.feed(Array("\u{1b}[31mhi\u{1b}[0m 世".utf8)) == TT_OK)
    #expect(session.update())
    #expect(session.view.cols == 10 && session.view.rows == 3)
    let h = try #require(session.cell(row: 0, col: 0))
    #expect(h.codepoint == UInt32(("h" as Unicode.Scalar).value))
    #expect(session.style(h.style).fg == UInt32(TT_COLOR_INDEXED) | 1)
    #expect(session.cell(row: 0, col: 3)?.width == 2)
    #expect(session.runs(row: 0).map(\.text) == ["hi", " ", "世"])
    #expect(session.cell(row: 0, col: 10) == nil)
    // No child: input is refused as closed rather than lost silently.
    #expect(session.text("x") == TT_CLOSED)
    #expect(session.scrollDisplay(1) == TT_OK)
}
