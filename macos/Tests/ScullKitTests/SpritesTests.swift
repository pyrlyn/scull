import CoreText
import Testing
@testable import ScullKit

private let cell = CellMetrics(font: CTFontCreateWithName("Menlo" as CFString, 13, nil),
                               cellWidth: 7.83, cellHeight: 16, scale: 2)

/// Coverage of one pixel, 0...255, top-down.
private func alpha(_ b: Bitmap, _ x: Int, _ y: Int) -> UInt8 { b.pixels[(y * b.width + x) * 4 + 3] }

private func sprite(_ code: UInt32) throws -> Bitmap {
    try #require(Sprites.bitmap(code, cell: cell))
}

@Test func everyBoxAndBlockCharacterHasASprite() throws {
    for code in UInt32(0x2500)...0x259F {
        let b = try sprite(code)
        #expect(b.width == 16 && b.height == 32 && b.top == Int(cell.baseline))
        #expect(stride(from: 3, to: b.pixels.count, by: 4).contains { b.pixels[$0] > 0 }, "U+\(String(code, radix: 16))")
    }
    #expect(Sprites.covers("─") && Sprites.covers("▟") && !Sprites.covers("a"))
    #expect(Sprites.bitmap(0x41, cell: cell) == nil)
    #expect(Sprites.bitmap(Sprites.underline(0), cell: cell) == nil)
}

/// Whether any pixel on each edge is inked: top, right, bottom, left.
private func edges(_ b: Bitmap) -> [Bool] {
    [(0..<b.width).contains { alpha(b, $0, 0) == 255 },
     (0..<b.height).contains { alpha(b, b.width - 1, $0) == 255 },
     (0..<b.width).contains { alpha(b, $0, b.height - 1) == 255 },
     (0..<b.height).contains { alpha(b, 0, $0) == 255 }]
}

@Test func lineArmsReachTheCellEdges() throws {
    #expect(try edges(sprite(0x253C)) == [true, true, true, true])
    #expect(try edges(sprite(0x250C)) == [false, true, true, false])
    #expect(try edges(sprite(0x2518)) == [true, false, false, true])
    #expect(try edges(sprite(0x2550)) == [false, true, false, true])
    #expect(try edges(sprite(0x256D)) == [false, true, true, false])
}

@Test func doubleLinesAreTwoLines() throws {
    let corner = try sprite(0x2554)
    let bottom = (0..<corner.width).map { alpha(corner, $0, corner.height - 1) > 0 }
    // Inked runs along the bottom edge: two separate lines.
    let runs = zip([false] + bottom, bottom).filter { !$0 && $1 }.count
    #expect(runs == 2)
    #expect(alpha(corner, 0, corner.height / 2) == 0)
}

@Test func blocksAndShadesCoverTheirShare() throws {
    let full = try sprite(0x2588)
    #expect(stride(from: 3, to: full.pixels.count, by: 4).allSatisfy { full.pixels[$0] == 255 })
    let half = try sprite(0x2584)
    #expect(alpha(half, 3, 2) == 0 && alpha(half, 3, half.height - 2) == 255)
    let shade = try sprite(0x2592)
    let mean = stride(from: 3, to: shade.pixels.count, by: 4).map { Int(shade.pixels[$0]) }.reduce(0, +)
        / (shade.width * shade.height)
    #expect((120...135).contains(mean))
}

@Test func underlineKindsDrawNearTheUnderlinePosition() throws {
    for kind in UInt8(1)...5 {
        let b = try sprite(Sprites.underline(kind))
        let inked = (0..<b.height).filter { y in (0..<b.width).contains { alpha(b, $0, y) > 0 } }
        let y = Int(cell.underline)
        #expect(!inked.isEmpty && inked.allSatisfy { abs($0 - y) <= 4 }, "kind \(kind): \(inked)")
    }
}

@Test func everyPowerlineSeparatorHasASprite() throws {
    for code in UInt32(0xE0B0)...0xE0BF {
        let b = try sprite(code)
        #expect(b.width == 16 && b.height == 32)
        #expect(stride(from: 3, to: b.pixels.count, by: 4).contains { b.pixels[$0] > 0 }, "U+\(String(code, radix: 16))")
    }
    #expect(Sprites.covers("\u{E0B0}") && Sprites.covers("\u{E0BF}") && !Sprites.covers("\u{E0C0}"))
}

/// Inked pixels of each column, left to right.
private func columns(_ b: Bitmap) -> [Int] {
    (0..<b.width).map { x in (0..<b.height).filter { alpha(b, x, $0) > 127 }.count }
}

@Test func solidPowerlineArrowsFillTheCellEdgeTheyStandOn() throws {
    let right = try sprite(0xE0B0), left = try sprite(0xE0B2)
    // The flat side covers its whole column, so the segment it ends has
    // no seam; the tip reaches the far edge at the middle row.
    #expect((1..<right.height - 1).allSatisfy { alpha(right, 0, $0) == 255 })
    #expect(alpha(right, right.width - 1, right.height / 2) > 0 && alpha(right, right.width - 1, 0) == 0)
    #expect(columns(left) == columns(right).reversed())
    let cap = try sprite(0xE0B4)
    #expect((1..<cap.height - 1).allSatisfy { alpha(cap, 0, $0) == 255 })
    #expect(columns(try sprite(0xE0B6)) == columns(cap).reversed())
}

@Test func powerlineOutlinesStayThinAndCornerTrianglesFillHalf() throws {
    let chevron = try sprite(0xE0B1)
    #expect(alpha(chevron, 0, chevron.height / 2) == 0 && alpha(chevron, chevron.width - 2, chevron.height / 2) > 0)
    for code in [UInt32(0xE0B8), 0xE0BA, 0xE0BC, 0xE0BE] {
        let b = try sprite(code)
        let ink = stride(from: 3, to: b.pixels.count, by: 4).map { Int(b.pixels[$0]) }.reduce(0, +)
        #expect((120...135).contains(ink / (b.width * b.height)), "U+\(String(code, radix: 16))")
    }
    let lowerLeft = try sprite(0xE0B8)
    #expect(alpha(lowerLeft, 1, lowerLeft.height - 2) == 255 && alpha(lowerLeft, lowerLeft.width - 2, 1) == 0)
}
