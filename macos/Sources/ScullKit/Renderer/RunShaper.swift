// Shapes the core's text runs with CoreText. CoreText brings ligatures and
// font fallback (colour emoji among them), but its advances belong to
// whichever font it fell back to; the core alone decides columns, so each
// glyph is pinned to the column of its cluster and keeps only its offset
// inside that cluster.

import CoreText
import Foundation

/// One glyph of a shaped run, placed relative to the run's first column.
public struct ShapedGlyph: Equatable, Sendable {
    public var key: AtlasKey
    /// Column of the glyph's cluster, counted from the run's first column.
    public var column: UInt16
    /// Points from the cluster's left edge and from the baseline (up).
    public var dx: Float, dy: Float
}

/// The fonts glyph keys refer to: the four faces first, then every
/// fallback font CoreText picks, numbered as they turn up.
public final class FontRegistry {
    public private(set) var fonts: [CTFont] = []
    private var ids: [String: Int32] = [:]

    public func id(of font: CTFont) -> Int32 {
        let name = "\(CTFontCopyPostScriptName(font) as String)@\(CTFontGetSize(font))"
        if let id = ids[name] { return id }
        let id = Int32(fonts.count)
        fonts.append(font)
        ids[name] = id
        return id
    }

    public func font(_ id: Int32) -> CTFont? {
        fonts.indices.contains(Int(id)) ? fonts[Int(id)] : nil
    }
}

public final class RunShaper {
    public let registry = FontRegistry()
    /// Regular, bold, italic and bold italic, by `face(bold:italic:)`.
    private let faces: [CTFont]
    /// Characters drawn from geometry rather than from the font.
    private let isSprite: (Unicode.Scalar) -> Bool
    private struct Key: Hashable {
        var text: String, face: UInt8, width: UInt8
    }
    // Two generations make a bounded cache that keeps what is still in
    // use: a hit in the old one moves forward, and the old one is dropped
    // whole when the new one fills.
    private var recent: [Key: [ShapedGlyph]] = [:]
    private var older: [Key: [ShapedGlyph]] = [:]
    let cacheLimit: Int

    public init(font: CTFont, cacheLimit: Int = 4096, isSprite: @escaping (Unicode.Scalar) -> Bool = { _ in false }) {
        let bold = CTFontCreateCopyWithSymbolicTraits(font, 0, nil, .traitBold, .traitBold) ?? font
        let italic = CTFontCreateCopyWithSymbolicTraits(font, 0, nil, .traitItalic, .traitItalic) ?? font
        let both = CTFontCreateCopyWithSymbolicTraits(bold, 0, nil, .traitItalic, .traitItalic) ?? bold
        faces = [font, bold, italic, both]
        (self.cacheLimit, self.isSprite) = (max(1, cacheLimit), isSprite)
        faces.forEach { _ = registry.id(of: $0) }
    }

    public static func face(bold: Bool, italic: Bool) -> Int { (bold ? 1 : 0) | (italic ? 2 : 0) }

    var cachedRuns: Int { recent.count + older.count }

    /// The glyphs of `text` in face `face` (0...3), each character taking
    /// `width` columns.
    public func shape(_ text: String, face: Int, width: Int) -> [ShapedGlyph] {
        let key = Key(text: text, face: UInt8(clamping: face), width: UInt8(clamping: width))
        if let hit = recent[key] { return hit }
        let glyphs = older.removeValue(forKey: key) ?? layout(text, face: faces[min(max(face, 0), 3)],
                                                              width: max(1, width))
        if recent.count >= cacheLimit { (older, recent) = (recent, [:]) }
        recent[key] = glyphs
        return glyphs
    }

    private func layout(_ text: String, face: CTFont, width: Int) -> [ShapedGlyph] {
        // Column and cluster start of every UTF-16 offset, as CoreText
        // reports string indices in UTF-16.
        let units = text.utf16.count
        var column = [Int](repeating: 0, count: units)
        var clusterStart = [Int](repeating: 0, count: units)
        var out: [ShapedGlyph] = []
        var sprites = Set<Int>()
        var offset = 0
        for (index, character) in text.enumerated() {
            let count = character.utf16.count
            for unit in offset..<offset + count {
                (column[unit], clusterStart[unit]) = (index * width, offset)
            }
            let scalars = character.unicodeScalars
            if scalars.count == 1, let scalar = scalars.first, isSprite(scalar) {
                sprites.insert(offset)
                out.append(ShapedGlyph(key: .sprite(scalar.value), column: UInt16(clamping: index * width),
                                       dx: 0, dy: 0))
            }
            offset += count
        }
        let string = NSAttributedString(string: text, attributes: [
            NSAttributedString.Key(kCTFontAttributeName as String): face,
            NSAttributedString.Key(kCTLigatureAttributeName as String): 1,
        ])
        let line = CTLineCreateWithAttributedString(string)
        var placed: [(key: AtlasKey, unit: Int, x: CGFloat, y: CGFloat)] = []
        // A cluster's origin is the pen position of its base character;
        // marks may be shifted left of it.
        var origin: [Int: (unit: Int, x: CGFloat)] = [:]
        for run in CTLineGetGlyphRuns(line) as? [CTRun] ?? [] {
            let count = CTRunGetGlyphCount(run)
            guard count > 0 else { continue }
            let value = (CTRunGetAttributes(run) as NSDictionary)[kCTFontAttributeName as String] as AnyObject?
            // The type is checked first, so the cast cannot fail.
            let runFont = value.flatMap { CFGetTypeID($0) == CTFontGetTypeID() ? ($0 as! CTFont) : nil } ?? face
            let font = registry.id(of: runFont)
            var glyphs = [CGGlyph](repeating: 0, count: count)
            var positions = [CGPoint](repeating: .zero, count: count)
            var indices = [CFIndex](repeating: 0, count: count)
            CTRunGetGlyphs(run, CFRange(), &glyphs)
            CTRunGetPositions(run, CFRange(), &positions)
            CTRunGetStringIndices(run, CFRange(), &indices)
            for i in 0..<count where indices[i] >= 0 && indices[i] < units && !sprites.contains(indices[i]) {
                let start = clusterStart[indices[i]]
                if origin[start].map({ indices[i] < $0.unit }) ?? true {
                    origin[start] = (indices[i], positions[i].x)
                }
                placed.append((.glyph(font: font, glyph: glyphs[i]), indices[i], positions[i].x, positions[i].y))
            }
        }
        for glyph in placed {
            let start = clusterStart[glyph.unit]
            out.append(ShapedGlyph(key: glyph.key, column: UInt16(clamping: column[glyph.unit]),
                                   dx: Float(glyph.x - (origin[start]?.x ?? glyph.x)), dy: Float(glyph.y)))
        }
        return out
    }
}
