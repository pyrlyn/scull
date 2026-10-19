// The glyph atlas: one texture of rasterised glyphs and sprites, packed in
// shelves. A terminal shows a few hundred distinct glyphs at a time, so
// shelves of near-equal height pack it well, and a whole shelf is the unit
// of eviction: cheap to track, and no free-rectangle bookkeeping.

import CoreGraphics
import CoreText
import Metal

/// What the atlas holds: a glyph of a registered font, or a sprite the
/// renderer draws from geometry (box drawing, underlines).
public enum AtlasKey: Hashable, Sendable {
    case glyph(font: Int32, glyph: CGGlyph)
    case sprite(UInt32)
}

/// A glyph's texels and how they sit against the pen position.
public struct AtlasEntry: Equatable, Sendable {
    public var x: UInt16 = 0, y: UInt16 = 0, width: UInt16 = 0, height: UInt16 = 0
    /// Pixels from the pen to the bitmap's left edge.
    public var left: Int16 = 0
    /// Pixels from the baseline up to the bitmap's top edge.
    public var top: Int16 = 0
    public var isColor = false
    var shelf: UInt16 = 0

    /// Nothing to draw (a space); remembered so it is not rasterised again.
    public var isEmpty: Bool { width == 0 }
}

/// A rasterised image before it enters the atlas: premultiplied BGRA rows,
/// top row first, the layout of the atlas texture.
public struct Bitmap: Sendable {
    public var width: Int, height: Int
    public var left: Int, top: Int
    public var isColor: Bool
    public var pixels: [UInt8]

    /// No glyph or sprite needs more; anything larger is a broken font.
    static let maxSide = 512

    /// Draws `body` into a fresh transparent bitmap with CoreGraphics'
    /// bottom-up coordinates.
    static func draw(width: Int, height: Int, left: Int, top: Int, isColor: Bool,
                     _ body: (CGContext) -> Void) -> Bitmap? {
        guard (1...maxSide).contains(width), (1...maxSide).contains(height),
              let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        let drawn = pixels.withUnsafeMutableBytes { raw -> Bool in
            let info = CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
            guard let ctx = CGContext(data: raw.baseAddress, width: width, height: height, bitsPerComponent: 8,
                                      bytesPerRow: width * 4, space: space, bitmapInfo: info) else { return false }
            body(ctx)
            return true
        }
        return drawn ? Bitmap(width: width, height: height, left: left, top: top, isColor: isColor, pixels: pixels) : nil
    }

    /// One glyph of `font` at `scale` pixels per point. Glyphs of a colour
    /// font (emoji) keep their colours; others become white coverage that
    /// the shader tints.
    public static func glyph(_ glyph: CGGlyph, font: CTFont, scale: CGFloat) -> Bitmap? {
        var glyph = glyph
        var bounds = CGRect.zero
        CTFontGetBoundingRectsForGlyphs(font, .default, &glyph, &bounds, 1)
        guard !bounds.isEmpty, bounds.width.isFinite, bounds.height.isFinite else { return nil }
        let isColor = CTFontGetSymbolicTraits(font).contains(.traitColorGlyphs)
        // One pixel of slack on each side: antialiasing spills past the
        // outline's bounds.
        let left = Int((bounds.minX * scale).rounded(.down)) - 1
        let bottom = Int((bounds.minY * scale).rounded(.down)) - 1
        let width = Int((bounds.maxX * scale).rounded(.up)) + 1 - left
        let height = Int((bounds.maxY * scale).rounded(.up)) + 1 - bottom
        return draw(width: width, height: height, left: left, top: bottom + height, isColor: isColor) { ctx in
            ctx.translateBy(x: CGFloat(-left), y: CGFloat(-bottom))
            ctx.scaleBy(x: scale, y: scale)
            ctx.setFillColor(CGColor(gray: 1, alpha: 1))
            var origin = CGPoint.zero
            CTFontDrawGlyphs(font, &glyph, &origin, 1, ctx)
        }
    }
}

public final class GlyphAtlas {
    private struct Shelf {
        var y: Int, height: Int, x = 0
        var lastUsed: UInt64
        var keys: [AtlasKey] = []
    }

    public private(set) var texture: MTLTexture
    public private(set) var size: Int
    /// Bumped when the atlas starts over at a larger size: every entry
    /// handed out before is gone, so rows built with them must be rebuilt.
    public private(set) var generation = 0
    private let device: MTLDevice
    private let maxSize: Int
    /// A shelf drawn within this many frames may still be read by the GPU,
    /// so it is not overwritten.
    private let framesInFlight: UInt64
    private var shelves: [Shelf] = []
    private var entries: [AtlasKey: AtlasEntry] = [:]
    private var nextY = 0

    public init?(device: MTLDevice, size: Int = 1024, maxSize: Int = 8192, framesInFlight: UInt64 = 3) {
        guard let texture = Self.makeTexture(device: device, size: size) else { return nil }
        (self.device, self.texture, self.size) = (device, texture, size)
        (self.maxSize, self.framesInFlight) = (max(size, maxSize), framesInFlight)
    }

    private static func makeTexture(device: MTLDevice, size: Int) -> MTLTexture? {
        let desc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .bgra8Unorm, width: size, height: size,
                                                            mipmapped: false)
        desc.storageMode = .shared
        desc.usage = .shaderRead
        return device.makeTexture(descriptor: desc)
    }

    /// The entry for `key`, rasterised by `make` the first time. Nil when
    /// the image fits nowhere even at the largest size; the caller skips it.
    public func entry(for key: AtlasKey, frame: UInt64, make: () -> Bitmap?) -> AtlasEntry? {
        if let entry = entries[key] {
            if !entry.isEmpty { shelves[Int(entry.shelf)].lastUsed = frame }
            return entry
        }
        guard let bitmap = make() else {
            entries[key] = AtlasEntry()
            return AtlasEntry()
        }
        return place(bitmap, key: key, frame: frame)
    }

    /// Marks a shelf as drawn in `frame`; rows reused from an earlier frame
    /// call this so their glyphs are not evicted under them.
    public func touch(shelf: UInt16, frame: UInt64) {
        guard Int(shelf) < shelves.count else { return }
        shelves[Int(shelf)].lastUsed = max(shelves[Int(shelf)].lastUsed, frame)
    }

    private func place(_ bitmap: Bitmap, key: AtlasKey, frame: UInt64) -> AtlasEntry? {
        let width = bitmap.width + 1
        // Rounded so glyphs of nearly the same height share shelves.
        let height = (bitmap.height + 1 + 3) / 4 * 4
        while width <= size, height <= size {
            if let index = shelfWithRoom(width: width, height: height) ?? newShelf(height: height, frame: frame)
                ?? evict(height: height, frame: frame) {
                return put(bitmap, key: key, shelf: index, width: width, frame: frame)
            }
            guard grow() else { break }
        }
        return nil
    }

    private func shelfWithRoom(width: Int, height: Int) -> Int? {
        shelves.indices.first { i in
            let shelf = shelves[i]
            return shelf.height >= height && shelf.height <= height + height / 4 + 4 && shelf.x + width <= size
        }
    }

    private func newShelf(height: Int, frame: UInt64) -> Int? {
        guard nextY + height <= size, shelves.count < Int(UInt16.max) else { return nil }
        shelves.append(Shelf(y: nextY, height: height, lastUsed: frame))
        nextY += height
        return shelves.count - 1
    }

    /// Empties the least recently drawn shelf tall enough for `height`.
    private func evict(height: Int, frame: UInt64) -> Int? {
        let candidates = shelves.indices.filter { i in
            shelves[i].height >= height && shelves[i].lastUsed + framesInFlight < frame
        }
        guard let index = candidates.min(by: { shelves[$0].lastUsed < shelves[$1].lastUsed }) else { return nil }
        for key in shelves[index].keys { entries[key] = nil }
        shelves[index].keys = []
        shelves[index].x = 0
        return index
    }

    private func grow() -> Bool {
        guard size * 2 <= maxSize, let bigger = Self.makeTexture(device: device, size: size * 2) else { return false }
        (texture, size) = (bigger, size * 2)
        shelves = []
        entries = [:]
        nextY = 0
        generation += 1
        return true
    }

    private func put(_ bitmap: Bitmap, key: AtlasKey, shelf index: Int, width: Int, frame: UInt64) -> AtlasEntry {
        let (x, y) = (shelves[index].x, shelves[index].y)
        bitmap.pixels.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            texture.replace(region: MTLRegionMake2D(x, y, bitmap.width, bitmap.height), mipmapLevel: 0,
                            withBytes: base, bytesPerRow: bitmap.width * 4)
        }
        shelves[index].x += width
        shelves[index].lastUsed = frame
        shelves[index].keys.append(key)
        let entry = AtlasEntry(x: UInt16(x), y: UInt16(y), width: UInt16(bitmap.width), height: UInt16(bitmap.height),
                               left: Int16(clamping: bitmap.left), top: Int16(clamping: bitmap.top),
                               isColor: bitmap.isColor, shelf: UInt16(index))
        entries[key] = entry
        return entry
    }
}
