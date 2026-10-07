// Image placements from the frame as textured quads. The core hands out
// decoded RGBA pixels; each image becomes a texture once and is reused
// while its placements stay on screen.

import CScull
import Metal

/// One placement ready to draw: its texture and its quad in view pixels.
struct PlacedImage {
    let texture: MTLTexture
    let quad: Quad
    let below: Bool
}

@MainActor
final class ImageLayer {
    private struct Key: Hashable {
        let image: UInt
        let generation: UInt64
    }

    private let device: MTLDevice
    private var textures: [Key: MTLTexture] = [:]

    init(device: MTLDevice) { self.device = device }

    /// The placements of `view` laid out on `metrics`' cells. Textures of
    /// images no longer placed are dropped: the frame lists every image
    /// in the viewport, and a command buffer still in flight keeps its own
    /// reference.
    func place(_ view: tt_frame_view, metrics: CellMetrics) -> [PlacedImage] {
        var kept: [Key: MTLTexture] = [:]
        var out: [PlacedImage] = []
        for p in UnsafeBufferPointer(start: view.placements, count: view.placements_len) {
            guard let image = p.image, p.cols > 0, p.rows > 0, p.src_w > 0, p.src_h > 0 else { continue }
            let key = Key(image: UInt(bitPattern: image), generation: p.generation)
            guard let texture = kept[key] ?? textures[key] ?? upload(image) else { continue }
            kept[key] = texture
            // The source rectangle stays inside the pixels, and within what
            // a quad's 16-bit texel fields can carry.
            let srcX = min(p.src_x, UInt32(texture.width - 1)), srcY = min(p.src_y, UInt32(texture.height - 1))
            let srcW = min(p.src_w, UInt32(texture.width) - srcX), srcH = min(p.src_h, UInt32(texture.height) - srcY)
            let x = (CGFloat(p.col) * metrics.width).rounded(.down) + CGFloat(p.offset_x)
            let y = (CGFloat(p.row) * metrics.height).rounded(.down) + CGFloat(p.offset_y)
            let quad = Quad(x: Float(x), y: Float(y), w: Float(CGFloat(p.cols) * metrics.width),
                            h: Float(CGFloat(p.rows) * metrics.height), u: UInt16(srcX), v: UInt16(srcY),
                            color: 0xFFFF_FFFF, kind: 3, su: UInt16(srcW), sv: UInt16(srcH))
            out.append(PlacedImage(texture: texture, quad: quad, below: p.z < 0))
        }
        textures = kept
        return out
    }

    private func upload(_ image: OpaquePointer) -> MTLTexture? {
        var (width, height, stride): (UInt32, UInt32, Int) = (0, 0, 0)
        guard let pixels = tt_image_pixels(image, &width, &height, &stride), width > 0, height > 0,
              width <= 16384, height <= 16384, stride >= Int(width) * 4 else { return nil }
        let desc = MTLTextureDescriptor.texture2DDescriptor(pixelFormat: .rgba8Unorm, width: Int(width),
                                                            height: Int(height), mipmapped: false)
        (desc.usage, desc.storageMode) = (.shaderRead, .shared)
        guard let texture = device.makeTexture(descriptor: desc) else { return nil }
        texture.replace(region: MTLRegionMake2D(0, 0, Int(width), Int(height)), mipmapLevel: 0,
                        withBytes: pixels, bytesPerRow: stride)
        return texture
    }
}
