// Draws a TerminalSession's frame with Metal: every cell background,
// glyph, sprite and decoration is one instanced quad. Rows are built on
// the CPU only when the frame's damage says they changed, kept in slots,
// and reached through a row-to-slot table, so a scroll only reorders the
// table. Each of the three in-flight buffers copies a slot only when its
// copy is older than the row.

import CScull
import CoreText
import Metal
import QuartzCore

/// One quad as the shaders read it; `Shaders.source` declares the same
/// layout.
struct Quad {
    var x: Float, y: Float, w: Float, h: Float
    var u: UInt16 = 0, v: UInt16 = 0
    var color: UInt32
    /// 0 solid, 1 coverage tinted by `color`, 2 colour glyph.
    var kind: UInt32 = 0
    var pad: UInt32 = 0
}

private struct Uniforms {
    var viewport: SIMD2<Float>
    var origin: SIMD2<Float>
}

private struct RowContent {
    var bg: [Quad] = [], fg: [Quad] = []
    var shelves: Set<UInt16> = []
    var version: UInt64 = 0
}

private struct InFlight {
    let buffer: MTLBuffer
    var versions: [UInt64]
}

@MainActor
public final class MetalRenderer {
    public let layer = CAMetalLayer()
    public var palette: Palette { didSet { needsRebuild = true } }
    private let device: MTLDevice
    private let queue: MTLCommandQueue
    private let pipeline: MTLRenderPipelineState
    private let font: CTFont
    private let cellSize: CGSize
    private let shaper: RunShaper
    private var atlas: GlyphAtlas
    private var scale: CGFloat = 0
    private var metrics: CellMetrics
    private var frame: UInt64 = 0
    private var version: UInt64 = 0
    private var grid = (cols: 0, rows: 0)
    private var rows: [RowContent] = []
    private var slotOfRow: [Int] = []
    private var buffers: [InFlight] = []
    private var nextBuffer = 0
    private var needsRebuild = true
    private var capacity = (bg: 0, fg: 0)
    private let inFlight = DispatchSemaphore(value: 3)

    /// `cellWidth` and `cellHeight` are the view's cell size in points.
    public init?(font: CTFont, cellWidth: CGFloat, cellHeight: CGFloat, palette: Palette) {
        guard let device = MTLCreateSystemDefaultDevice(), let queue = device.makeCommandQueue(),
              let pipeline = try? Self.pipeline(device), let atlas = GlyphAtlas(device: device) else { return nil }
        (self.device, self.queue, self.pipeline, self.atlas) = (device, queue, pipeline, atlas)
        (self.font, self.palette) = (font, palette)
        cellSize = CGSize(width: cellWidth, height: cellHeight)
        metrics = CellMetrics(font: font, cellWidth: cellWidth, cellHeight: cellHeight, scale: 1)
        shaper = RunShaper(font: font, isSprite: Sprites.covers)
        layer.device = device
        layer.pixelFormat = .bgra8Unorm
        layer.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        layer.isOpaque = true
        layer.maximumDrawableCount = 3
    }

    private static func pipeline(_ device: MTLDevice) throws -> MTLRenderPipelineState {
        let library = try device.makeLibrary(source: Shaders.source, options: nil)
        let desc = MTLRenderPipelineDescriptor()
        desc.vertexFunction = library.makeFunction(name: "quad_vertex")
        desc.fragmentFunction = library.makeFunction(name: "quad_fragment")
        let color = desc.colorAttachments[0]
        color?.pixelFormat = .bgra8Unorm
        color?.isBlendingEnabled = true
        color?.sourceRGBBlendFactor = .one
        color?.sourceAlphaBlendFactor = .one
        color?.destinationRGBBlendFactor = .oneMinusSourceAlpha
        color?.destinationAlphaBlendFactor = .oneMinusSourceAlpha
        return try device.makeRenderPipelineState(descriptor: desc)
    }

    /// Draws the session's latest frame into `layer`, `size` in points.
    public func draw(_ session: TerminalSession, size: CGSize, scale: CGFloat, focused: Bool) {
        prepare(session, scale: scale)
        let pixels = CGSize(width: (size.width * scale).rounded(), height: (size.height * scale).rounded())
        guard pixels.width >= 1, pixels.height >= 1 else { return }
        if layer.drawableSize != pixels { layer.drawableSize = pixels }
        layer.contentsScale = scale
        guard let drawable = layer.nextDrawable() else { return }
        _ = encode(session, into: drawable.texture, focused: focused) { $0.present(drawable) }
    }

    /// The same picture rendered offscreen, for tests and the debug
    /// snapshot.
    public func snapshot(_ session: TerminalSession, size: CGSize, scale: CGFloat, focused: Bool) -> CGImage? {
        prepare(session, scale: scale)
        let (w, h) = (Int((size.width * scale).rounded()), Int((size.height * scale).rounded()))
        guard w > 0, h > 0 else { return nil }
        let desc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .bgra8Unorm, width: w, height: h,
                                                            mipmapped: false)
        (desc.usage, desc.storageMode) = (.renderTarget, .shared)
        guard let texture = device.makeTexture(descriptor: desc),
              let commands = encode(session, into: texture, focused: focused, beforeCommit: { _ in }) else { return nil }
        commands.waitUntilCompleted()
        var pixels = [UInt8](repeating: 0, count: w * h * 4)
        texture.getBytes(&pixels, bytesPerRow: w * 4, from: MTLRegionMake2D(0, 0, w, h), mipmapLevel: 0)
        let info = CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        guard let space = CGColorSpace(name: CGColorSpace.sRGB),
              let provider = CGDataProvider(data: Data(pixels) as CFData) else { return nil }
        return CGImage(width: w, height: h, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: w * 4, space: space,
                       bitmapInfo: CGBitmapInfo(rawValue: info), provider: provider, decode: nil,
                       shouldInterpolate: false, intent: .defaultIntent)
    }

    // MARK: Rows

    private func prepare(_ session: TerminalSession, scale: CGFloat) {
        if scale != self.scale, scale > 0 {
            self.scale = scale
            metrics = CellMetrics(font: font, cellWidth: cellSize.width, cellHeight: cellSize.height, scale: scale)
            // Glyphs rasterised at the old scale are the wrong size.
            atlas = GlyphAtlas(device: device) ?? atlas
            needsRebuild = true
        }
        let view = session.view
        let damage = session.takeDamage()
        if Int(view.cols) != grid.cols || Int(view.rows) != grid.rows {
            allocate(cols: Int(view.cols), rows: Int(view.rows))
        }
        frame += 1
        var dirty = Array(0..<grid.rows)
        if !needsRebuild, damage.sources.count == grid.rows { dirty = remap(damage.sources) }
        needsRebuild = false
        // Shelves that reused rows draw from must survive this frame's
        // evictions.
        for slot in slotOfRow { rows[slot].shelves.forEach { atlas.touch(shelf: $0, frame: frame) } }
        var generation = atlas.generation
        dirty.forEach { build(row: $0, session) }
        // A grown atlas dropped every entry, so rows built before the
        // growth point at texels that are gone.
        for _ in 0..<4 where atlas.generation != generation {
            generation = atlas.generation
            (0..<grid.rows).forEach { build(row: $0, session) }
        }
    }

    private func allocate(cols: Int, rows count: Int) {
        grid = (cols, count)
        // A row has at most one background per cell; the cap on the rest
        // keeps a flood of combining marks from growing the buffers.
        capacity = (cols, cols * 4 + 16)
        rows = Array(repeating: RowContent(), count: count)
        slotOfRow = Array(0..<count)
        let length = max(1, count * (capacity.bg + capacity.fg)) * MemoryLayout<Quad>.stride
        buffers = (0..<3).compactMap { _ in
            device.makeBuffer(length: length, options: .storageModeShared)
                .map { InFlight(buffer: $0, versions: Array(repeating: 0, count: count)) }
        }
        needsRebuild = true
    }

    /// Moves reused rows' slots to their new rows; answers the rows that
    /// got a free slot and must be built.
    private func remap(_ sources: [Int?]) -> [Int] {
        var taken = [Bool](repeating: false, count: slotOfRow.count)
        var next = [Int](repeating: -1, count: sources.count)
        for (row, source) in sources.enumerated() {
            guard let source, source < slotOfRow.count, !taken[slotOfRow[source]] else { continue }
            next[row] = slotOfRow[source]
            taken[slotOfRow[source]] = true
        }
        var free = taken.indices.filter { !taken[$0] }
        var dirty: [Int] = []
        for row in next.indices where next[row] < 0 {
            next[row] = free.removeLast()
            dirty.append(row)
        }
        slotOfRow = next
        return dirty
    }

    private func x(_ col: Int) -> Float { Float((CGFloat(col) * metrics.width).rounded(.down)) }

    private static func rgba(_ rgb: UInt32, alpha: UInt32 = 255) -> UInt32 {
        (rgb >> 16 & 0xFF) | (rgb & 0xFF00) | (rgb & 0xFF) << 16 | alpha << 24
    }

    private func entry(_ key: AtlasKey) -> AtlasEntry? {
        atlas.entry(for: key, frame: frame) {
            switch key {
            case let .glyph(font, glyph): shaper.registry.font(font).flatMap { Bitmap.glyph(glyph, font: $0, scale: scale) }
            case let .sprite(code): Sprites.bitmap(code, cell: metrics)
            }
        }
    }

    private func build(row: Int, _ session: TerminalSession) {
        var content = RowContent()
        // Neighbouring cells of one colour share a quad.
        var start = 0, current = palette.background
        let flush = { (end: Int, content: inout RowContent) in
            guard current != self.palette.background, end > start else { return }
            content.bg.append(Quad(x: self.x(start), y: 0, w: self.x(end) - self.x(start),
                                   h: Float(self.metrics.height), color: Self.rgba(current)))
        }
        for col in 0..<grid.cols {
            guard let cell = session.cell(row: row, col: col) else { break }
            let bg = palette.colors(of: session.style(cell.style)).bg
            if bg != current {
                flush(col, &content)
                (start, current) = (col, bg)
            }
        }
        flush(grid.cols, &content)
        for (run, text) in session.runs(row: row) {
            addRun(run, text: text, style: session.style(run.style), to: &content)
        }
        version += 1
        content.version = version
        rows[slotOfRow[row]] = content
    }

    private func addRun(_ run: tt_run, text: String, style: tt_style, to content: inout RowContent) {
        let attrs = style.attrs
        guard attrs & UInt16(TT_ATTR_HIDDEN) == 0 else { return }
        let fg = palette.colors(of: style).fg
        let alpha: UInt32 = attrs & UInt16(TT_ATTR_DIM) != 0 ? 128 : 255
        let color = Self.rgba(fg, alpha: alpha)
        let face = RunShaper.face(bold: attrs & UInt16(TT_ATTR_BOLD) != 0, italic: attrs & UInt16(TT_ATTR_ITALIC) != 0)
        for glyph in shaper.shape(text, face: face, width: Int(run.width)) where content.fg.count < capacity.fg {
            guard let entry = entry(glyph.key), !entry.isEmpty else { continue }
            let col = CGFloat(Int(run.col) + Int(glyph.column)) * metrics.width
            // Sprites start on the cell's pixel edge, as backgrounds do.
            let isSprite = if case .sprite = glyph.key { true } else { false }
            let pen = isSprite ? col.rounded(.down) : (col + CGFloat(glyph.dx) * scale).rounded()
            let baseline = metrics.baseline - (CGFloat(glyph.dy) * scale).rounded()
            content.fg.append(Quad(x: Float(pen) + Float(entry.left), y: Float(baseline) - Float(entry.top),
                                   w: Float(entry.width), h: Float(entry.height), u: entry.x, v: entry.y,
                                   color: entry.isColor ? Self.rgba(0xFFFFFF, alpha: alpha) : color,
                                   kind: entry.isColor ? 2 : 1))
            content.shelves.insert(entry.shelf)
        }
        let (first, end) = (Int(run.col), Int(run.col) + Int(run.cols))
        if style.underline != UInt8(TT_UNDERLINE_NONE), let entry = entry(.sprite(Sprites.underline(style.underline))),
           !entry.isEmpty {
            let tint = style.underline_color == UInt32(TT_COLOR_DEFAULT) ? color
                : Self.rgba(palette.rgb(style.underline_color, fallback: fg), alpha: alpha)
            for col in first..<end where content.fg.count < capacity.fg {
                content.fg.append(Quad(x: x(col), y: Float(metrics.baseline) - Float(entry.top), w: Float(entry.width),
                                       h: Float(entry.height), u: entry.x, v: entry.y, color: tint, kind: 1))
            }
            content.shelves.insert(entry.shelf)
        }
        let line = { (y: CGFloat) in
            Quad(x: self.x(first), y: Float(y.rounded()), w: self.x(end) - self.x(first),
                 h: Float(self.metrics.thickness), color: color)
        }
        if attrs & UInt16(TT_ATTR_STRIKE) != 0, content.fg.count < capacity.fg {
            content.fg.append(line(metrics.baseline - CTFontGetXHeight(font) * scale / 2))
        }
        if attrs & UInt16(TT_ATTR_OVERLINE) != 0, content.fg.count < capacity.fg { content.fg.append(line(0)) }
    }

    // MARK: Encoding

    private func encode(_ session: TerminalSession, into target: MTLTexture, focused: Bool,
                        beforeCommit: (MTLCommandBuffer) -> Void) -> MTLCommandBuffer? {
        inFlight.wait()
        guard let commands = queue.makeCommandBuffer() else {
            inFlight.signal()
            return nil
        }
        commands.addCompletedHandler { [inFlight] _ in inFlight.signal() }
        let pass = MTLRenderPassDescriptor()
        pass.colorAttachments[0].texture = target
        pass.colorAttachments[0].loadAction = .clear
        pass.colorAttachments[0].storeAction = .store
        let bg = palette.background
        pass.colorAttachments[0].clearColor = MTLClearColor(red: Double(bg >> 16 & 0xFF) / 255,
                                                            green: Double(bg >> 8 & 0xFF) / 255,
                                                            blue: Double(bg & 0xFF) / 255, alpha: 1)
        guard let encoder = commands.makeRenderCommandEncoder(descriptor: pass) else {
            commands.commit()
            return commands
        }
        encoder.setRenderPipelineState(pipeline)
        encoder.setFragmentTexture(atlas.texture, index: 0)
        var uniforms = Uniforms(viewport: SIMD2(Float(target.width), Float(target.height)), origin: .zero)
        if !buffers.isEmpty {
            let index = nextBuffer
            nextBuffer = (nextBuffer + 1) % buffers.count
            upload(into: index)
            let stride = capacity.bg + capacity.fg
            let (cursor, overlay) = cursorQuads(session, focused: focused)
            for layer in 0..<2 {
                encoder.setVertexBuffer(buffers[index].buffer, offset: 0, index: 0)
                for row in 0..<grid.rows {
                    let slot = slotOfRow[row]
                    let count = layer == 0 ? rows[slot].bg.count : rows[slot].fg.count
                    guard count > 0 else { continue }
                    uniforms.origin = SIMD2(0, Float(CGFloat(row) * metrics.height))
                    encoder.setVertexBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 1)
                    encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4, instanceCount: count,
                                           baseInstance: slot * stride + (layer == 0 ? 0 : capacity.bg))
                }
                // The block goes over the backgrounds; the glyph under it
                // is drawn again on top in the background colour.
                draw(layer == 0 ? cursor : overlay, encoder: encoder, uniforms: &uniforms)
            }
        }
        encoder.endEncoding()
        beforeCommit(commands)
        commands.commit()
        return commands
    }

    private func draw(_ quads: [Quad], encoder: MTLRenderCommandEncoder, uniforms: inout Uniforms) {
        guard !quads.isEmpty else { return }
        uniforms.origin = .zero
        encoder.setVertexBytes(quads, length: quads.count * MemoryLayout<Quad>.stride, index: 0)
        encoder.setVertexBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 1)
        encoder.drawPrimitives(type: .triangleStrip, vertexStart: 0, vertexCount: 4, instanceCount: quads.count)
    }

    private func upload(into index: Int) {
        let stride = capacity.bg + capacity.fg
        let base = buffers[index].buffer.contents().bindMemory(to: Quad.self, capacity: grid.rows * stride)
        for slot in rows.indices where buffers[index].versions[slot] != rows[slot].version {
            for (quads, offset) in [(rows[slot].bg, 0), (rows[slot].fg, capacity.bg)] {
                quads.withUnsafeBufferPointer { source in
                    guard let from = source.baseAddress else { return }
                    (base + slot * stride + offset).update(from: from, count: source.count)
                }
            }
            buffers[index].versions[slot] = rows[slot].version
        }
    }

    /// The cursor's own quads in view pixels: what goes under the text,
    /// and what goes over it.
    private func cursorQuads(_ session: TerminalSession, focused: Bool) -> (under: [Quad], over: [Quad]) {
        let cursor = session.view.cursor
        let (row, col) = (Int(cursor.row), Int(cursor.col))
        guard cursor.visible != 0, row < grid.rows, let cell = session.cell(row: row, col: col) else { return ([], []) }
        let top = Float(CGFloat(row) * metrics.height)
        let (left, right) = (x(col), x(col + max(1, Int(cell.width))))
        let (height, t) = (Float(metrics.height), Float(metrics.thickness))
        let color = Self.rgba(palette.cursor)
        guard focused else {
            return ([Quad(x: left, y: top, w: right - left, h: t, color: color),
                     Quad(x: left, y: top + height - t, w: right - left, h: t, color: color),
                     Quad(x: left, y: top, w: t, h: height, color: color),
                     Quad(x: right - t, y: top, w: t, h: height, color: color)], [])
        }
        let over = rows[slotOfRow[row]].fg.filter { quad in
            quad.kind == 1 && quad.x + quad.w / 2 >= left && quad.x + quad.w / 2 < right
        }.prefix(16).map { quad in
            var quad = quad
            quad.y += top
            quad.color = Self.rgba(palette.background)
            return quad
        }
        return ([Quad(x: left, y: top, w: right - left, h: height, color: color)], Array(over))
    }
}
