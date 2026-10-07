import CoreText
import Metal
import Testing
@testable import ScullKit

private func device() throws -> MTLDevice {
    try #require(MTLCreateSystemDefaultDevice())
}

/// A solid bitmap whose blue channel tells tests which one it is.
private func square(_ side: Int, height: Int? = nil, tag: UInt8 = 0xFF) -> Bitmap {
    let height = height ?? side
    var pixels = [UInt8](repeating: 0xFF, count: side * height * 4)
    for i in stride(from: 0, to: pixels.count, by: 4) { pixels[i] = tag }
    return Bitmap(width: side, height: height, left: 0, top: height, isColor: false, pixels: pixels)
}

/// Wider than half the atlas, so each one takes a 16-texel shelf of its own.
private func shelfFiller() -> Bitmap { square(40, height: 14) }

private func texel(_ atlas: GlyphAtlas, x: Int, y: Int) -> [UInt8] {
    var out = [UInt8](repeating: 0, count: 4)
    atlas.texture.getBytes(&out, bytesPerRow: 4, from: MTLRegionMake2D(x, y, 1, 1), mipmapLevel: 0)
    return out
}

@Test func atlasRasterisesOnceAndUploadsThePixels() throws {
    let atlas = try #require(GlyphAtlas(device: device(), size: 64, maxSize: 64))
    var made = 0
    let make = { () -> Bitmap? in made += 1; return square(8, tag: 0x42) }
    let first = try #require(atlas.entry(for: .sprite(1), frame: 1, make: make))
    let again = try #require(atlas.entry(for: .sprite(1), frame: 2, make: make))
    #expect(first == again && made == 1)
    #expect(texel(atlas, x: Int(first.x) + 3, y: Int(first.y) + 3) == [0x42, 0xFF, 0xFF, 0xFF])
}

@Test func atlasEntriesNeverOverlap() throws {
    let atlas = try #require(GlyphAtlas(device: device(), size: 128, maxSize: 128))
    var rects: [CGRect] = []
    for i in 0..<60 {
        let side = 6 + i % 9
        let entry = try #require(atlas.entry(for: .sprite(UInt32(i)), frame: 1) { square(side) })
        let rect = CGRect(x: Int(entry.x), y: Int(entry.y), width: Int(entry.width), height: Int(entry.height))
        #expect(rect.maxX <= 128 && rect.maxY <= 128)
        #expect(!rects.contains { $0.intersects(rect) })
        rects.append(rect)
    }
}

@Test func atlasRemembersGlyphsWithNothingToDraw() throws {
    let atlas = try #require(GlyphAtlas(device: device(), size: 64))
    var made = 0
    let make = { () -> Bitmap? in made += 1; return nil }
    #expect(atlas.entry(for: .sprite(32), frame: 1, make: make)?.isEmpty == true)
    #expect(atlas.entry(for: .sprite(32), frame: 2, make: make)?.isEmpty == true)
    #expect(made == 1)
}

@Test func atlasEvictsTheLeastRecentlyDrawnShelf() throws {
    // Four shelves of 16 rows fill a 64-texel atlas.
    let atlas = try #require(GlyphAtlas(device: device(), size: 64, maxSize: 64, framesInFlight: 2))
    for shelf in 0..<4 {
        _ = try #require(atlas.entry(for: .sprite(UInt32(shelf)), frame: 1) { shelfFiller() })
    }
    // Shelves 0, 2 and 3 are drawn again later; shelf 1 is the oldest.
    for shelf in [0, 2, 3] { _ = atlas.entry(for: .sprite(UInt32(shelf)), frame: 5) { nil } }
    let fresh = try #require(atlas.entry(for: .sprite(9), frame: 6) { shelfFiller() })
    #expect(fresh.y == 16)
    var remade = false
    _ = atlas.entry(for: .sprite(1), frame: 6) { remade = true; return shelfFiller() }
    #expect(remade)
    #expect(atlas.generation == 0)
}

@Test func atlasNeverEvictsAShelfAFrameInFlightMayRead() throws {
    let atlas = try #require(GlyphAtlas(device: device(), size: 64, maxSize: 64, framesInFlight: 3))
    for shelf in 0..<4 {
        _ = try #require(atlas.entry(for: .sprite(UInt32(shelf)), frame: 10) { shelfFiller() })
    }
    #expect(atlas.entry(for: .sprite(9), frame: 12) { shelfFiller() } == nil)
    #expect(atlas.entry(for: .sprite(9), frame: 14) { shelfFiller() } != nil)
}

@Test func atlasGrowsWhenNothingCanBeEvicted() throws {
    let atlas = try #require(GlyphAtlas(device: device(), size: 64, maxSize: 128))
    for shelf in 0..<4 {
        _ = try #require(atlas.entry(for: .sprite(UInt32(shelf)), frame: 1) { shelfFiller() })
    }
    let entry = try #require(atlas.entry(for: .sprite(9), frame: 1) { shelfFiller() })
    #expect(atlas.size == 128 && atlas.texture.width == 128)
    #expect(atlas.generation == 1)
    // Growth starts over, so the new image is the first one in.
    #expect(entry.x == 0 && entry.y == 0)
}

@Test func glyphBitmapsKeepColourOnlyForColourFonts() throws {
    let text = CTFontCreateWithName("Menlo" as CFString, 13, nil)
    var glyph: CGGlyph = 0
    var a: UniChar = 0x41
    #expect(CTFontGetGlyphsForCharacters(text, &a, &glyph, 1))
    let letter = try #require(Bitmap.glyph(glyph, font: text, scale: 2))
    #expect(!letter.isColor && letter.top > 0 && letter.width > 4)
    // Coverage is white: every inked pixel has equal channels.
    let inked = stride(from: 0, to: letter.pixels.count, by: 4).filter { letter.pixels[$0 + 3] > 0 }
    #expect(!inked.isEmpty)
    #expect(inked.allSatisfy { letter.pixels[$0] == letter.pixels[$0 + 3] })

    let emoji = CTFontCreateWithName("Apple Color Emoji" as CFString, 13, nil)
    var pair: [UniChar] = Array("😀".utf16)
    var glyphs = [CGGlyph](repeating: 0, count: 2)
    #expect(CTFontGetGlyphsForCharacters(emoji, &pair, &glyphs, 2))
    let face = try #require(Bitmap.glyph(glyphs[0], font: emoji, scale: 2))
    #expect(face.isColor)
    let coloured = stride(from: 0, to: face.pixels.count, by: 4).contains {
        face.pixels[$0] != face.pixels[$0 + 2] && face.pixels[$0 + 3] == 0xFF
    }
    #expect(coloured)

    var space: UniChar = 0x20
    #expect(CTFontGetGlyphsForCharacters(text, &space, &glyph, 1))
    #expect(Bitmap.glyph(glyph, font: text, scale: 2) == nil)
}
