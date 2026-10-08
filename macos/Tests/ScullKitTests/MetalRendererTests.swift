import CoreText
import Foundation
import CScull
import Testing
@testable import ScullKit

@MainActor
@Test func damageComposesAcrossUpdatesAndScrolls() throws {
    let session = try TerminalSession(cols: 4, rows: 3)
    _ = session.feed(Array("1\r\n2\r\n3".utf8))
    session.update()
    #expect(session.takeDamage().sources == [nil, nil, nil])
    #expect(session.takeDamage().sources == [0, 1, 2])
    _ = session.feed(Array("\r\n4".utf8))
    session.update()
    _ = session.feed(Array("\r\n5".utf8))
    session.update()
    // "3" was row 2 of the drawn picture; "4" and "5" were never drawn.
    #expect(session.takeDamage().sources == [2, nil, nil])
}

/// 8 x 16 point cells at scale 2: 16 x 32 pixels.
private let cellPixels = (w: 16, h: 32)

@MainActor
private func render(_ session: TerminalSession, _ renderer: MetalRenderer, focused: Bool = true) throws -> [UInt8] {
    session.update()
    let size = CGSize(width: Double(session.view.cols) * 8, height: Double(session.view.rows) * 16)
    let image = try #require(renderer.snapshot(session, size: size, scale: 2, focused: focused))
    let data = try #require(image.dataProvider?.data as Data?)
    #expect(image.width == Int(session.view.cols) * cellPixels.w && image.bytesPerRow == image.width * 4)
    return [UInt8](data)
}

/// The sRGB colour at a pixel inside a cell, as 0xRRGGBB.
private func rgb(_ pixels: [UInt8], cols: Int, row: Int, col: Int, dx: Int, dy: Int) -> UInt32 {
    let (x, y) = (col * cellPixels.w + dx, row * cellPixels.h + dy)
    let i = (y * cols * cellPixels.w + x) * 4
    return UInt32(pixels[i + 2]) << 16 | UInt32(pixels[i + 1]) << 8 | UInt32(pixels[i])
}

@MainActor
private func renderer() throws -> MetalRenderer {
    try #require(MetalRenderer(font: CTFontCreateWithName("Menlo" as CFString, 13, nil), cellWidth: 8, cellHeight: 16,
                               palette: Palette()))
}

@MainActor
@Test func rendererDrawsBackgroundsSpritesAndTheCursor() throws {
    let session = try TerminalSession(cols: 6, rows: 2)
    let renderer = try renderer()
    _ = session.feed(Array("\u{1b}[41mA\u{1b}[0m ─█".utf8))
    let pixels = try render(session, renderer)
    let at = { (row: Int, col: Int, dx: Int, dy: Int) in rgb(pixels, cols: 6, row: row, col: col, dx: dx, dy: dy) }
    #expect(at(0, 0, 1, 1) == 0xCD0000)
    #expect(at(0, 1, 8, 16) == 0x141414)
    // The light line crosses the cell's middle from edge to edge.
    #expect([0, 15].allSatisfy { dx in (14...17).contains { at(0, 2, dx, $0) == 0xE5E5E5 } })
    #expect(at(0, 3, 0, 0) == 0xE5E5E5 && at(0, 3, 15, 31) == 0xE5E5E5)
    // The focused cursor is a block on the next cell; unfocused, a frame.
    #expect(at(0, 4, 8, 16) == 0xE5E5E5)
    let hollow = try render(session, renderer, focused: false)
    #expect(rgb(hollow, cols: 6, row: 0, col: 4, dx: 8, dy: 16) == 0x141414)
    #expect(rgb(hollow, cols: 6, row: 0, col: 4, dx: 0, dy: 16) == 0xE5E5E5)
}

@MainActor
@Test func rendererDrawsTextAndColourEmoji() throws {
    let session = try TerminalSession(cols: 6, rows: 1)
    let renderer = try renderer()
    _ = session.feed(Array("\u{1b}[32mW\u{1b}[0m😀".utf8))
    let pixels = try render(session, renderer)
    let cell = { (col: Int) in
        (0..<cellPixels.h).flatMap { dy in (0..<cellPixels.w).map { rgb(pixels, cols: 6, row: 0, col: col, dx: $0, dy: dy) } }
    }
    #expect(cell(0).contains(0x00CD00))
    // The emoji keeps its own colours: some pixel is far from grey.
    let emoji = cell(1) + cell(2)
    #expect(emoji.contains { abs(Int($0 >> 16) - Int($0 & 0xFF)) > 80 })
}

@MainActor
@Test func rendererKeepsRowsRightAcrossScrollsAndRepaints() throws {
    let session = try TerminalSession(cols: 3, rows: 3)
    let renderer = try renderer()
    _ = session.feed(Array("█\r\n\r\n\u{1b}[44m \u{1b}[0m".utf8))
    var pixels = try render(session, renderer)
    #expect(rgb(pixels, cols: 3, row: 0, col: 0, dx: 4, dy: 4) == 0xE5E5E5)
    #expect(rgb(pixels, cols: 3, row: 2, col: 0, dx: 4, dy: 4) == 0x0000EE)
    // One line feed scrolls the block and the blue cell up one row.
    _ = session.feed(Array("\r\nx".utf8))
    pixels = try render(session, renderer)
    #expect(rgb(pixels, cols: 3, row: 0, col: 0, dx: 4, dy: 4) == 0x141414)
    #expect(rgb(pixels, cols: 3, row: 1, col: 0, dx: 4, dy: 4) == 0x0000EE)
    #expect(rgb(pixels, cols: 3, row: 2, col: 0, dx: 1, dy: 1) == 0x141414)
    _ = session.feed(Array("\u{1b}[H█".utf8))
    pixels = try render(session, renderer)
    #expect(rgb(pixels, cols: 3, row: 0, col: 0, dx: 4, dy: 4) == 0xE5E5E5)
    #expect(rgb(pixels, cols: 3, row: 1, col: 0, dx: 4, dy: 4) == 0x0000EE)
}

@MainActor
@Test func rendererDrawsImagePlacementsAboveAndBelowText() throws {
    let session = try TerminalSession(cols: 4, rows: 2)
    let renderer = try renderer()
    // The core sizes placements in cells from the cell's pixel size.
    #expect(session.resize(cols: 4, rows: 2, widthPx: 64, heightPx: 64) == TT_OK)
    let red = "/wAA//8AAP//AAD//wAA/w=="
    // 2 x 2 red pixels scaled to one cell; then the same image under text
    // on the next cell, which keeps showing the text over it.
    _ = session.feed(Array("\u{1b}_Gf=32,s=2,v=2,a=T,i=1,c=1,r=1;\(red)\u{1b}\\".utf8))
    _ = session.feed(Array("\u{1b}[1;2H\u{1b}_Ga=p,i=1,c=1,r=1,z=-1,C=1\u{1b}\\█".utf8))
    let pixels = try render(session, renderer)
    #expect(session.view.placements_len == 2)
    #expect(rgb(pixels, cols: 4, row: 0, col: 0, dx: 8, dy: 16) == 0xFF0000)
    #expect(rgb(pixels, cols: 4, row: 0, col: 1, dx: 8, dy: 16) == 0xE5E5E5)
    // A deleted image is no longer drawn.
    _ = session.feed(Array("\u{1b}_Ga=d,d=A\u{1b}\\".utf8))
    let cleared = try render(session, renderer)
    #expect(rgb(cleared, cols: 4, row: 0, col: 0, dx: 8, dy: 16) == 0x141414)
}

@MainActor
@Test func rendererUnderlinesThePreeditSpan() throws {
    let session = try TerminalSession(cols: 6, rows: 1)
    let renderer = try renderer()
    // Spaces leave only the underline in the cells it marks.
    #expect(session.preedit("  ", caret: 2) == TT_OK)
    let pixels = try render(session, renderer)
    let inked = { (col: Int) in (16..<32).contains { rgb(pixels, cols: 6, row: 0, col: col, dx: 8, dy: $0) == 0xE5E5E5 } }
    #expect(session.view.preedit.cols == 2)
    #expect(inked(0) && inked(1))
    #expect(!inked(4))
}

@MainActor
@Test func rendererRemakesGlyphsForANewFont() throws {
    let session = try TerminalSession(cols: 2, rows: 1)
    let renderer = try renderer()
    _ = session.feed(Array("█".utf8))
    _ = try render(session, renderer)
    // Twice the cell: the block sprite cached at the old size must not be
    // reused.
    renderer.setFont(CTFontCreateWithName("Menlo" as CFString, 26, nil), cellWidth: 16, cellHeight: 32)
    let image = try #require(renderer.snapshot(session, size: CGSize(width: 32, height: 32), scale: 2, focused: false))
    let data = [UInt8](try #require(image.dataProvider?.data as Data?))
    let at = { (x: Int, y: Int) in data[(y * image.width + x) * 4 + 2] }
    #expect(image.width == 64)
    #expect(at(30, 60) == 0xE5 && at(20, 40) == 0xE5)
    #expect(at(40, 30) == 0x14)
}

@MainActor
@Test func rendererShadesSelectedAndMatchedCellsWhenOnlyTheirFlagsChange() throws {
    let session = try TerminalSession(cols: 8, rows: 2)
    let renderer = try renderer()
    _ = session.feed(Array("ab ab\r\nxy".utf8))
    _ = try render(session, renderer)
    // Neither text nor style changes below, only the frame's flags, so the
    // rows are rebuilt from damage alone.
    #expect(session.select(TT_SELECT_CELL, row: 1, col: 0) == TT_OK)
    #expect(session.extendSelection(row: 1, col: 1) == TT_OK)
    #expect(session.search("ab", ignoreCase: false) == TT_OK)
    #expect(session.searchStep(forward: true) != nil)
    let pixels = try render(session, renderer)
    let palette = Palette()
    let at = { (row: Int, col: Int) in rgb(pixels, cols: 8, row: row, col: col, dx: 0, dy: 1) }
    #expect(at(0, 0) == palette.currentMatch && at(0, 1) == palette.currentMatch)
    #expect(at(0, 3) == palette.match && at(0, 2) == palette.background)
    #expect(at(1, 0) == palette.selection && at(1, 1) == palette.selection && at(1, 3) == palette.background)
    // The current match's text is drawn dark on its bright background.
    let ink = (0..<cellPixels.h).flatMap { dy in (0..<cellPixels.w).map { rgb(pixels, cols: 8, row: 0, col: 0, dx: $0, dy: dy) } }
    #expect(ink.contains(palette.currentMatchText) && !ink.contains(palette.foreground))
    _ = session.clearSelection()
    _ = session.search("", ignoreCase: false)
    let cleared = try render(session, renderer)
    #expect(rgb(cleared, cols: 8, row: 0, col: 0, dx: 0, dy: 1) == palette.background)
    #expect(rgb(cleared, cols: 8, row: 1, col: 0, dx: 0, dy: 1) == palette.background)
}

@Test func paletteLayersCellMarksAndBlends() {
    let palette = Palette()
    let (selected, match, current) = (UInt8(TT_CELL_SELECTED), UInt8(TT_CELL_MATCH), UInt8(TT_CELL_CURRENT_MATCH))
    #expect(palette.background(flags: 0, bg: 0x123456) == 0x123456)
    #expect(palette.background(flags: match, bg: 0x123456) == palette.match)
    #expect(palette.background(flags: selected | match, bg: 0) == palette.selection)
    #expect(palette.background(flags: selected | match | current, bg: 0) == palette.currentMatch)
    #expect(Palette.blend(0xFF0080, over: 0x000000, alpha: 0.5) == 0x800040)
    #expect(Palette.blend(0x0A0B0C, over: 0xFFFFFF, alpha: 1) == 0x0A0B0C)
}
