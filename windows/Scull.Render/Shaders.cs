using System.Text;
using Windows.Win32;
using Windows.Win32.Graphics.Direct3D;

namespace Scull.Render;

/// <summary>
/// The renderer's HLSL, compiled from source with <c>D3DCompile</c> when a
/// renderer starts, as the Metal shaders are on macOS (Shaders.swift). Not
/// precompiled bytecode: fxc runs only on Windows, so a committed blob could
/// not be rebuilt or diff-checked from macOS or Linux. d3dcompiler_47.dll is
/// in System32 on the Windows CI runner; Microsoft documents the SDK's copy as
/// a redistributable rather than a system component, so the packaged app
/// (T17.3) may have to ship it beside the executable (research.md §9).
/// </summary>
internal static unsafe class Shaders
{
    // The per-instance layout is `Quad` in Renderer.cs.
    private const string Source = """
        cbuffer Uniforms : register(b0) { float2 viewport; float2 origin; };

        struct Instance {
            float4 rect : RECT;   // x, y, width, height in pixels from the origin
            uint2 uv : UV;        // atlas texel of the top-left corner
            float4 color : COLOR; // straight alpha
            uint kind : KIND;     // 0 solid, 1 tinted coverage, 2 colour glyph
        };

        struct Fragment {
            float4 position : SV_Position;
            float2 uv : TEXCOORD0;
            float4 color : COLOR;
            nointerpolation uint kind : KIND;
        };

        Fragment vs(uint vid : SV_VertexID, Instance q) {
            float2 corner = float2(vid & 1, vid >> 1);
            float2 p = origin + q.rect.xy + corner * q.rect.zw;
            Fragment o;
            o.position = float4(p / viewport * float2(2, -2) + float2(-1, 1), 0, 1);
            o.uv = float2(q.uv) + corner * q.rect.zw;
            o.color = q.color;
            o.kind = q.kind;
            return o;
        }

        Texture2D<float4> atlas : register(t0);

        // Output is premultiplied; the blend is one, one minus source alpha.
        // Quads sit on whole pixels, so a texel fetch needs no sampler.
        float4 ps(Fragment f) : SV_Target {
            if (f.kind == 0) return float4(f.color.rgb * f.color.a, f.color.a);
            float4 t = atlas.Load(int3(f.uv, 0));
            if (f.kind == 2) return t * f.color.a;
            float a = t.a * f.color.a;
            return float4(f.color.rgb * a, a);
        }
        """;

    /// <summary>Bytecode of <paramref name="entry"/> for <paramref name="target"/>; the caller releases it.</summary>
    public static ID3DBlob* Compile(string entry, string target)
    {
        ID3DBlob* code = null;
        ID3DBlob* errors = null;
        var status = PInvoke.D3DCompile(Encoding.UTF8.GetBytes(Source), "scull.hlsl", null, null, entry, target,
            PInvoke.D3DCOMPILE_ENABLE_STRICTNESS | PInvoke.D3DCOMPILE_OPTIMIZATION_LEVEL3, 0, &code, &errors);
        string message = errors == null ? "" : Encoding.UTF8.GetString((byte*)errors->GetBufferPointer(), (int)errors->GetBufferSize());
        Com.Release(ref errors);
        if (status.Failed)
        {
            Com.Release(ref code);
            throw new InvalidOperationException($"{entry}: {message}");
        }
        return code;
    }
}
