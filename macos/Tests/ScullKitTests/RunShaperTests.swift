import AppKit
import CoreText
import Testing
@testable import ScullKit

private var menlo: CTFont { CTFontCreateWithName("Menlo" as CFString, 13, nil) }

private func fonts(_ shaper: RunShaper, _ glyphs: [ShapedGlyph]) -> [CTFont] {
    glyphs.compactMap { glyph in
        if case let .glyph(font, _) = glyph.key { return shaper.registry.font(font) }
        return nil
    }
}

@Test func shaperKeepsLigatures() {
    // Helvetica Neue has an "fi" ligature; terminal fonts with "->" ligatures work
    // the same way but may not be installed.
    let shaper = RunShaper(font: CTFontCreateWithName("Helvetica Neue" as CFString, 13, nil))
    let glyphs = shaper.shape("fi", face: 0, width: 1)
    #expect(glyphs.count == 1)
    #expect(glyphs.first?.column == 0)
}

@Test func shaperFallsBackToColourEmoji() throws {
    let shaper = RunShaper(font: menlo)
    let glyphs = shaper.shape("😀", face: 0, width: 2)
    let font = try #require(fonts(shaper, glyphs).first)
    #expect(CTFontGetSymbolicTraits(font).contains(.traitColorGlyphs))
    #expect(glyphs.map(\.column) == [0])
}

@Test func shaperPinsGlyphsToTheirClusterColumns() {
    let shaper = RunShaper(font: menlo)
    let marks = shaper.shape("x\u{301}y", face: 0, width: 1)
    #expect(marks.map(\.column) == [0, 0, 1])
    #expect(marks[0].dx == 0 && marks[2].dx == 0)
    let wide = shaper.shape("世界", face: 0, width: 2)
    #expect(wide.map(\.column) == [0, 2])
    #expect(wide.allSatisfy { $0.dx == 0 })
}

@Test func shaperPicksBoldAndItalicFaces() {
    let shaper = RunShaper(font: menlo)
    let bold = fonts(shaper, shaper.shape("a", face: RunShaper.face(bold: true, italic: false), width: 1))
    let italic = fonts(shaper, shaper.shape("a", face: RunShaper.face(bold: false, italic: true), width: 1))
    #expect(bold.first.map { CTFontGetSymbolicTraits($0).contains(.traitBold) } == true)
    #expect(italic.first.map { CTFontGetSymbolicTraits($0).contains(.traitItalic) } == true)
}

@Test func shaperHandsSpriteCharactersToTheSpriteDrawer() {
    let shaper = RunShaper(font: menlo) { $0.value == 0x2500 }
    let glyphs = shaper.shape("a─b", face: 0, width: 1)
    #expect(glyphs.contains { $0.key == .sprite(0x2500) && $0.column == 1 })
    #expect(glyphs.filter { $0.column == 1 }.count == 1)
    #expect(glyphs.count == 3)
}

@Test func shapedRunCacheStaysBounded() {
    let shaper = RunShaper(font: menlo, cacheLimit: 4)
    let first = shaper.shape("hello", face: 0, width: 1)
    for i in 0..<20 { _ = shaper.shape("run \(i)", face: 0, width: 1) }
    #expect(shaper.cachedRuns <= 8)
    #expect(shaper.shape("hello", face: 0, width: 1) == first)
}

@Test func shaperDrawsPowerlineWithTheBundledSymbols() {
    let url = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().appendingPathComponent("../../Resources/\(SymbolFont.file).ttf")
    #expect(SymbolFont.register(url))
    let shaper = RunShaper(font: NSFont.monospacedSystemFont(ofSize: 13, weight: .regular))
    let names = fonts(shaper, shaper.shape("a\u{E0B0}b", face: 0, width: 1))
        .map { CTFontCopyPostScriptName($0) as String }
    #expect(names.count == 3 && names[1] == SymbolFont.name && names[0] == names[2])
}
