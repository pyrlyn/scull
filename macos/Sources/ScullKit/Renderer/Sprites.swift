// Characters drawn from geometry instead of the font: box drawing, block
// elements, Powerline separators and the underline kinds. A font's own box glyphs rarely fill
// the cell exactly, so lines break between rows; drawn to the cell's own
// size they always meet.

import CoreGraphics
import CoreText

/// Cell geometry in pixels, shared by the renderer and the sprites.
public struct CellMetrics: Equatable, Sendable {
    /// Cell size; the width may be fractional, as the view's grid is.
    public var width: CGFloat, height: CGFloat
    /// From the cell's top to the baseline.
    public var baseline: CGFloat
    /// From the cell's top to the middle of the underline.
    public var underline: CGFloat
    /// A light line.
    public var thickness: CGFloat

    public init(font: CTFont, cellWidth: CGFloat, cellHeight: CGFloat, scale: CGFloat) {
        (width, height) = (cellWidth * scale, cellHeight * scale)
        baseline = (CTFontGetAscent(font) * scale).rounded()
        thickness = max(1, (CTFontGetUnderlineThickness(font) * scale).rounded())
        underline = min(baseline - CTFontGetUnderlinePosition(font) * scale, height - thickness * 2)
    }
}

public enum Sprites {
    /// Sprite codes past Unicode's last code point, one per `TT_UNDERLINE_*`.
    public static func underline(_ kind: UInt8) -> UInt32 { 0x11_0000 + UInt32(kind) }

    /// Box drawing and block elements, then the Powerline separators.
    private static let ranges: [ClosedRange<UInt32>] = [0x2500...0x259F, 0xE0B0...0xE0BF]

    public static func covers(_ scalar: Unicode.Scalar) -> Bool { ranges.contains { $0.contains(scalar.value) } }

    /// The sprite for `code`, or nil when it is none.
    public static func bitmap(_ code: UInt32, cell: CellMetrics) -> Bitmap? {
        guard ranges.contains(where: { $0.contains(code) }) || (underline(1)...underline(5)).contains(code)
        else { return nil }
        let (w, h) = (Int(cell.width.rounded(.up)), Int(cell.height.rounded(.up)))
        return Bitmap.draw(width: w, height: h, left: 0, top: Int(cell.baseline), isColor: false) { ctx in
            // Top-down, like the cell rows.
            ctx.translateBy(x: 0, y: CGFloat(h))
            ctx.scaleBy(x: 1, y: -1)
            ctx.setFillColor(CGColor(gray: 1, alpha: 1))
            ctx.setStrokeColor(CGColor(gray: 1, alpha: 1))
            Painter(ctx: ctx, w: CGFloat(w), h: CGFloat(h), cell: cell).paint(code)
        }
    }
}

/// Arms of U+2500...U+257F as up, right, down, left: 0 none, 1 light,
/// 2 heavy, 3 double. Dashes, arcs and diagonals are drawn apart (0000).
private let arms: [UInt8] = Array(("""
    0101 0202 1010 2020 0000 0000 0000 0000 0000 0000 0000 0000 0110 0210 0120 0220
    0011 0012 0021 0022 1100 1200 2100 2200 1001 1002 2001 2002 1110 1210 2110 1120
    2120 2210 1220 2220 1011 1012 2011 1021 2021 2012 1022 2022 0111 0112 0211 0212
    0121 0122 0221 0222 1101 1102 1201 1202 2101 2102 2201 2202 1111 1112 1211 1212
    2111 1121 2121 2112 2211 1122 1221 2212 1222 2122 2221 2222 0000 0000 0000 0000
    0303 3030 0310 0130 0330 0013 0031 0033 1300 3100 3300 1003 3001 3003 1310 3130
    3330 1013 3031 3033 0313 0131 0333 1303 3101 3303 1313 3131 3333 0000 0000 0000
    0000 0000 0000 0000 0001 1000 0100 0010 0002 2000 0200 0020 0201 1020 0102 2010
    """).utf8.compactMap { $0 >= 0x30 && $0 <= 0x33 ? $0 - 0x30 : nil })

/// Quadrants of U+2596...U+259F: 1 upper left, 2 upper right, 4 lower
/// left, 8 lower right.
private let quadrants: [Int] = [4, 8, 1, 13, 9, 7, 11, 2, 6, 14]

private struct Painter {
    let ctx: CGContext
    let w: CGFloat, h: CGFloat
    let cell: CellMetrics
    var t: CGFloat { cell.thickness }
    var cx: CGFloat { (w / 2).rounded(.down) }
    var cy: CGFloat { (h / 2).rounded(.down) }

    func paint(_ code: UInt32) {
        switch code {
        case 0x2504...0x250B: dashes(code - 0x2504, segments: code < 0x2508 ? 3 : 4)
        case 0x254C...0x254F: dashes(code - 0x254C, segments: 2)
        case 0x256D...0x2570: arc(code)
        case 0x2571...0x2573: diagonals(code)
        case 0x2500...0x257F: lines(Array(arms[Int(code - 0x2500) * 4..<Int(code - 0x2500) * 4 + 4]))
        case 0x2580: fill(0, 0, 1, 0.5)
        case 0x2581...0x2588: fill(0, 1 - CGFloat(code - 0x2580) / 8, 1, 1)
        case 0x2589...0x258F: fill(0, 0, CGFloat(0x2590 - code) / 8, 1)
        case 0x2590: fill(0.5, 0, 1, 1)
        case 0x2591...0x2593:
            // Shades as even coverage, not a dither that beats against
            // the pixel grid.
            ctx.setFillColor(CGColor(gray: 1, alpha: CGFloat(code - 0x2590) / 4))
            fill(0, 0, 1, 1)
        case 0x2594: fill(0, 0, 1, 1 / 8)
        case 0x2595: fill(7 / 8, 0, 1, 1)
        case 0x2596...0x259F:
            let mask = quadrants[Int(code - 0x2596)]
            if mask & 1 != 0 { fill(0, 0, 0.5, 0.5) }
            if mask & 2 != 0 { fill(0.5, 0, 1, 0.5) }
            if mask & 4 != 0 { fill(0, 0.5, 0.5, 1) }
            if mask & 8 != 0 { fill(0.5, 0.5, 1, 1) }
        case 0xE0B0...0xE0BF: powerline(code)
        default: underline(UInt8(truncatingIfNeeded: code - Sprites.underline(0)))
        }
    }

    private func fill(_ x0: CGFloat, _ y0: CGFloat, _ x1: CGFloat, _ y1: CGFloat) {
        let (left, top) = ((x0 * w).rounded(), (y0 * h).rounded())
        ctx.fill(CGRect(x: left, y: top, width: (x1 * w).rounded() - left, height: (y1 * h).rounded() - top))
    }

    /// A pixel-aligned bar of thickness `th` centred `offset` off the cell's
    /// middle, from `start` past the centre (negative: short of it) to the
    /// edge in `direction` (0 up, 1 right, 2 down, 3 left).
    private func bar(_ direction: Int, start: CGFloat, offset: CGFloat, thickness th: CGFloat) {
        let horizontal = direction % 2 == 1
        let across = ((horizontal ? cy : cx) + offset - th / 2).rounded(.down)
        let middle = horizontal ? cx : cy
        let (from, to) = direction == 1 || direction == 2 ? (middle + start, horizontal ? w : h) : (0, middle - start)
        let rect = horizontal ? CGRect(x: from, y: across, width: to - from, height: th)
            : CGRect(x: across, y: from, width: th, height: to - from)
        ctx.fill(rect.standardized)
    }

    private func lines(_ arm: [UInt8]) {
        let d = t
        let weight = { (dir: Int) -> CGFloat in arm[dir] == 2 ? t * 2 : t }
        let widest = (0..<4).map { arm[$0] == 3 ? d * 2 + t : weight($0) }.max() ?? t
        for dir in 0..<4 where arm[dir] != 0 {
            // The perpendicular arms, on the side of a negative offset first.
            let opposite = arm[(dir + 2) % 4], sides = dir % 2 == 1 ? [arm[0], arm[2]] : [arm[3], arm[1]]
            if arm[dir] == 3 {
                // Each of the two lines stops at a double line crossing it,
                // runs on into a continuing arm, meets a single line at the
                // centre, and otherwise turns the outer corner.
                for (i, side) in sides.enumerated() {
                    let other = sides[1 - i]
                    let start: CGFloat = side == 3 ? d
                        : opposite != 0 ? -d - t / 2
                        : (side == 1 || side == 2 || other == 1 || other == 2) ? -t / 2 : -d - t / 2
                    bar(dir, start: start, offset: i == 0 ? -d : d, thickness: t)
                }
            } else {
                // A stem off a double line stops at its near line, or at the
                // far one where the double line turns a corner.
                let start: CGFloat = opposite == 0 && sides.contains(3)
                    ? (sides.allSatisfy { $0 != 0 } ? d : -d) : -widest / 2
                bar(dir, start: start, offset: 0, thickness: weight(dir))
            }
        }
    }

    private func dashes(_ index: UInt32, segments: Int) {
        let vertical = index % 4 >= 2
        let th = index % 2 == 1 ? t * 2 : t
        let slot = (vertical ? h : w) / CGFloat(segments)
        let gap = max(1, (slot / 3).rounded())
        for i in 0..<segments {
            let from = (CGFloat(i) * slot + gap / 2).rounded()
            let to = (CGFloat(i + 1) * slot - gap / 2).rounded()
            ctx.fill(vertical ? CGRect(x: (cx - th / 2).rounded(.down), y: from, width: th, height: to - from)
                : CGRect(x: from, y: (cy - th / 2).rounded(.down), width: to - from, height: th))
        }
    }

    private func arc(_ code: UInt32) {
        // The stroke's centre line sits where a light bar's centre does.
        let (px, py) = ((cx - t / 2).rounded(.down) + t / 2, (cy - t / 2).rounded(.down) + t / 2)
        let down = code == 0x256D || code == 0x256E
        let right = code == 0x256D || code == 0x2570
        let path = CGMutablePath()
        path.move(to: CGPoint(x: px, y: down ? h : 0))
        path.addArc(tangent1End: CGPoint(x: px, y: py), tangent2End: CGPoint(x: right ? w : 0, y: py),
                    radius: max(1, min(w, h) / 2 - t))
        path.addLine(to: CGPoint(x: right ? w : 0, y: py))
        stroke(path)
    }

    private func diagonals(_ code: UInt32) {
        let path = CGMutablePath()
        if code != 0x2572 { path.addLines(between: [CGPoint(x: w, y: 0), CGPoint(x: 0, y: h)]) }
        if code != 0x2571 { path.addLines(between: [CGPoint(x: 0, y: 0), CGPoint(x: w, y: h)]) }
        stroke(path)
    }

    /// Powerline separators: even codes are solid, odd ones their outline.
    /// The solid shapes reach the cell edges, so they meet the background
    /// of the next segment without a seam.
    private func powerline(_ code: UInt32) {
        let solid = code % 2 == 0
        let path = CGMutablePath()
        let point = { (x: CGFloat, y: CGFloat) in CGPoint(x: x * w, y: y * h) }
        switch code {
        case 0xE0B0, 0xE0B1: path.addLines(between: [point(0, 0), point(1, 0.5), point(0, 1)])
        case 0xE0B2, 0xE0B3: path.addLines(between: [point(1, 0), point(0, 0.5), point(1, 1)])
        case 0xE0B4...0xE0B7:
            // Half an ellipse bulging away from the edge it stands on; the
            // outline is inset so the whole stroke stays in the cell.
            let inset = solid ? 0 : t / 2
            let right = code < 0xE0B6
            path.addEllipse(in: CGRect(x: right ? -w : inset, y: inset, width: w * 2 - inset, height: h - inset * 2))
        case 0xE0B8: path.addLines(between: [point(0, 0), point(1, 1), point(0, 1)])
        case 0xE0BA: path.addLines(between: [point(1, 0), point(1, 1), point(0, 1)])
        case 0xE0BC: path.addLines(between: [point(0, 0), point(1, 0), point(0, 1)])
        case 0xE0BE: path.addLines(between: [point(0, 0), point(1, 0), point(1, 1)])
        // The outlines of the corner triangles are their hypotenuses.
        case 0xE0B9, 0xE0BF: path.addLines(between: [point(0, 0), point(1, 1)])
        default: path.addLines(between: [point(1, 0), point(0, 1)])
        }
        if solid {
            path.closeSubpath()
            ctx.addPath(path)
            ctx.fillPath()
        } else {
            stroke(path)
        }
    }

    private func stroke(_ path: CGPath) {
        ctx.addPath(path)
        ctx.setLineWidth(t)
        ctx.strokePath()
    }

    private func underline(_ kind: UInt8) {
        let y = cell.underline
        switch kind {
        case 2:
            ctx.fill(CGRect(x: 0, y: (y - t * 1.5).rounded(), width: w, height: t))
            ctx.fill(CGRect(x: 0, y: (y + t / 2).rounded(), width: w, height: t))
        case 3:
            // One period per cell, so neighbouring cells join.
            let path = CGMutablePath()
            let amplitude = max(1, t)
            path.move(to: CGPoint(x: 0, y: y))
            for x in stride(from: 1, through: w, by: 1) {
                path.addLine(to: CGPoint(x: x, y: y - amplitude * sin(2 * .pi * x / w)))
            }
            stroke(path)
        case 4:
            for x in stride(from: 0, to: w, by: t * 2) {
                ctx.fill(CGRect(x: x, y: (y - t / 2).rounded(), width: t, height: t))
            }
        case 5: ctx.fill(CGRect(x: (w * 0.15).rounded(), y: (y - t / 2).rounded(), width: (w * 0.7).rounded(), height: t))
        default: ctx.fill(CGRect(x: 0, y: (y - t / 2).rounded(), width: w, height: t))
        }
    }
}
