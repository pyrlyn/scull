// The renderer's shaders. They are compiled from source when the renderer
// starts because SwiftPM does not build `.metal` files; it takes a few
// milliseconds, once.

enum Shaders {
    static let source = """
    #include <metal_stdlib>
    using namespace metal;

    // Same layout as `Quad` in MetalRenderer.swift.
    struct Quad {
        float4 rect;   // x, y, width, height in pixels from the origin
        ushort2 uv;    // atlas texel of the top-left corner
        uint color;    // RGBA8, red in the low byte
        uint kind;     // 0 solid, 1 tinted coverage, 2 colour glyph, 3 image
        ushort2 span;  // texels an image samples
    };

    struct Uniforms {
        float2 viewport;
        float2 origin;
    };

    struct Fragment {
        float4 position [[position]];
        float2 uv;
        float4 color;
        uint kind [[flat]];
    };

    vertex Fragment quad_vertex(uint vid [[vertex_id]], uint iid [[instance_id]],
                                const device Quad *quads [[buffer(0)]],
                                constant Uniforms &u [[buffer(1)]]) {
        Quad q = quads[iid];
        float2 corner = float2(vid & 1, vid >> 1);
        float2 p = u.origin + q.rect.xy + corner * q.rect.zw;
        Fragment out;
        out.position = float4(p / u.viewport * float2(2, -2) + float2(-1, 1), 0, 1);
        out.uv = float2(q.uv) + corner * (q.kind == 3 ? float2(q.span) : q.rect.zw);
        out.color = unpack_unorm4x8_to_float(q.color);
        out.kind = q.kind;
        return out;
    }

    // Output is premultiplied; the blend is one, one minus source alpha.
    fragment float4 quad_fragment(Fragment in [[stage_in]], texture2d<float> atlas [[texture(0)]],
                                  texture2d<float> image [[texture(1)]]) {
        constexpr sampler texel(coord::pixel, filter::nearest);
        constexpr sampler scaled(coord::pixel, filter::linear, address::clamp_to_edge);
        if (in.kind == 3) {
            // Image pixels have straight alpha.
            float4 t = image.sample(scaled, in.uv);
            return float4(t.rgb * t.a, t.a);
        }
        if (in.kind == 0) {
            return float4(in.color.rgb * in.color.a, in.color.a);
        }
        float4 t = atlas.sample(texel, in.uv);
        if (in.kind == 2) {
            return t * in.color.a;
        }
        float a = t.a * in.color.a;
        return float4(in.color.rgb * a, a);
    }
    """
}
