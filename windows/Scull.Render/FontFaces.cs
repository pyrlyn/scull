using System.Runtime.Versioning;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.Graphics.DirectWrite;

namespace Scull.Render;

/// <summary>Pixel metrics of one cell, rounded so cell edges land on whole pixels.</summary>
public readonly record struct CellMetrics(int Width, int Height, int Baseline, int Underline, int Strike, int Thickness);

/// <summary>A rasterised glyph before it enters the atlas: premultiplied BGRA rows, top first.</summary>
internal ref struct GlyphBitmap
{
    public int Width, Height, Left, Top;
    public bool IsColor;
    public Span<byte> Pixels;
}

/// <summary>
/// The font in its four styles plus a short fallback list, through
/// DirectWrite: glyph lookup by code point and rasterisation as grayscale
/// coverage or, for colour fonts, COLR layers composited here. Shaping and the
/// system fallback chain come with T17.5.
/// </summary>
// IDWriteFactory2, for grayscale analysis and colour layers, is Windows 8.1's.
[SupportedOSPlatform("windows8.1")]
internal sealed unsafe class FontFaces : IDisposable
{
    // Fallback families tried, in order, when the face lacks a code point;
    // a missing family is skipped.
    private static readonly string[] Fallbacks = ["Segoe UI Emoji", "Segoe UI Symbol"];

    // No glyph needs more; anything larger is a broken font.
    private const int MaxSide = 512;

    private IDWriteFactory2* factory;
    private readonly nint[] faces;
    private readonly float emSize;
    private byte[] coverage = new byte[64 * 64];
    private byte[] pixels = new byte[64 * 64 * 4];
    private readonly nint[] layers = new nint[16];
    private readonly DWRITE_COLOR_F[] layerColors = new DWRITE_COLOR_F[16];

    public FontFaces(string family, float emSizePixels)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(emSizePixels, 1f);
        emSize = emSizePixels;
        PInvoke.DWriteCreateFactory(DWRITE_FACTORY_TYPE.DWRITE_FACTORY_TYPE_SHARED, out IDWriteFactory2* f).ThrowOnFailure();
        factory = f;
        IDWriteFontCollection* fonts;
        factory->GetSystemFontCollection(&fonts, false);
        try
        {
            var found = new List<nint>(4 + Fallbacks.Length);
            foreach ((DWRITE_FONT_WEIGHT weight, DWRITE_FONT_STYLE style) in new[]
            {
                (DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_NORMAL),
                (DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_NORMAL),
                (DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_ITALIC),
                (DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_ITALIC),
            })
            {
                found.Add((nint)Face(fonts, family, weight, style) is var face and not 0 ? face
                    : throw new ArgumentException($"no font family named {family}", nameof(family)));
            }
            foreach (string fallback in Fallbacks)
            {
                if (Face(fonts, fallback, DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_NORMAL) is var face && face != null)
                {
                    found.Add((nint)face);
                }
            }
            faces = [.. found];
        }
        finally
        {
            fonts->Release();
        }
        Metrics = MeasureCell();
    }

    public CellMetrics Metrics { get; }

    private static IDWriteFontFace* Face(IDWriteFontCollection* fonts, string name, DWRITE_FONT_WEIGHT weight, DWRITE_FONT_STYLE style)
    {
        uint index;
        BOOL exists;
        fixed (char* n = name)
        {
            fonts->FindFamilyName(n, &index, &exists);
        }
        if (!exists)
        {
            return null;
        }
        IDWriteFontFamily* family;
        IDWriteFont* font;
        IDWriteFontFace* face;
        fonts->GetFontFamily(index, &family);
        family->GetFirstMatchingFont(weight, DWRITE_FONT_STRETCH.DWRITE_FONT_STRETCH_NORMAL, style, &font);
        font->CreateFontFace(&face);
        font->Release();
        family->Release();
        return face;
    }

    private CellMetrics MeasureCell()
    {
        var face = (IDWriteFontFace*)faces[0];
        DWRITE_FONT_METRICS m;
        face->GetMetrics(&m);
        float scale = emSize / m.designUnitsPerEm;
        uint zero = '0';
        ushort glyph;
        face->GetGlyphIndices(&zero, 1, &glyph);
        DWRITE_GLYPH_METRICS g;
        face->GetDesignGlyphMetrics(&glyph, 1, &g, false);
        int ascent = (int)MathF.Ceiling(m.ascent * scale);
        int descent = (int)MathF.Ceiling(m.descent * scale);
        int gap = (int)MathF.Round(m.lineGap * scale);
        int baseline = ascent + gap / 2;
        // Font metrics put both lines' positions above the baseline, so a
        // negative underline position is below it.
        return new CellMetrics(Math.Max(1, (int)MathF.Round(g.advanceWidth * scale)), ascent + descent + gap, baseline,
            baseline - (int)MathF.Round(m.underlinePosition * scale), baseline - (int)MathF.Round(m.strikethroughPosition * scale),
            Math.Max(1, (int)MathF.Round(m.underlineThickness * scale)));
    }

    /// <summary>The glyph for <paramref name="codePoint"/> in the styled face, else in the first fallback that has it.</summary>
    public GlyphKey Lookup(uint codePoint, bool bold, bool italic)
    {
        int styled = (bold ? 1 : 0) | (italic ? 2 : 0);
        ushort glyph;
        ((IDWriteFontFace*)faces[styled])->GetGlyphIndices(&codePoint, 1, &glyph);
        for (int i = 4; glyph == 0 && i < faces.Length; i++)
        {
            ((IDWriteFontFace*)faces[i])->GetGlyphIndices(&codePoint, 1, &glyph);
            if (glyph != 0)
            {
                return new GlyphKey((ushort)i, glyph);
            }
        }
        // Glyph 0 of the face is its "missing" box, which is what should show.
        return new GlyphKey((ushort)styled, glyph);
    }

    /// <summary>Rasterises <paramref name="key"/>; false when it draws nothing.</summary>
    public bool Rasterize(GlyphKey key, out GlyphBitmap bitmap)
    {
        ushort glyph = key.Glyph;
        float advance = 0;
        var run = new DWRITE_GLYPH_RUN
        {
            fontFace = (IDWriteFontFace*)faces[key.Face],
            fontEmSize = emSize,
            glyphCount = 1,
            glyphIndices = &glyph,
            glyphAdvances = &advance,
        };
        var white = new DWRITE_COLOR_F { r = 1, g = 1, b = 1, a = 1 };
        IDWriteColorGlyphRunEnumerator* enumerator;
        if (factory->TranslateColorGlyphRun(0, 0, &run, null, DWRITE_MEASURING_MODE.DWRITE_MEASURING_MODE_NATURAL, null, 0, &enumerator).Failed)
        {
            // DWRITE_E_NOCOLOR: a plain glyph, white coverage the shader tints.
            layers[0] = (nint)Analyse(&run, 0, 0, out RECT bounds);
            layerColors[0] = white;
            return Draw(1, bounds, isColor: false, out bitmap);
        }
        int count = 0;
        var canvas = new RECT { left = int.MaxValue, top = int.MaxValue, right = int.MinValue, bottom = int.MinValue };
        BOOL has;
        for (enumerator->MoveNext(&has); has && count < layers.Length; enumerator->MoveNext(&has))
        {
            DWRITE_COLOR_GLYPH_RUN* layer;
            enumerator->GetCurrentRun(&layer);
            layers[count] = (nint)Analyse(&layer->glyphRun, layer->baselineOriginX, layer->baselineOriginY, out RECT b);
            // Palette index 0xFFFF means the text colour; the atlas keeps it white.
            layerColors[count++] = layer->paletteIndex == 0xFFFF ? white : layer->runColor;
            canvas = new RECT
            {
                left = Math.Min(canvas.left, b.left),
                top = Math.Min(canvas.top, b.top),
                right = Math.Max(canvas.right, b.right),
                bottom = Math.Max(canvas.bottom, b.bottom),
            };
        }
        enumerator->Release();
        return Draw(count, canvas, isColor: true, out bitmap);
    }

    private IDWriteGlyphRunAnalysis* Analyse(DWRITE_GLYPH_RUN* run, float x, float y, out RECT bounds)
    {
        IDWriteGlyphRunAnalysis* analysis;
        // Grayscale: one atlas serves every background colour, which
        // ClearType's per-channel coverage cannot.
        factory->CreateGlyphRunAnalysis(run, null, DWRITE_RENDERING_MODE.DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
            DWRITE_MEASURING_MODE.DWRITE_MEASURING_MODE_NATURAL, DWRITE_GRID_FIT_MODE.DWRITE_GRID_FIT_MODE_DEFAULT,
            DWRITE_TEXT_ANTIALIAS_MODE.DWRITE_TEXT_ANTIALIAS_MODE_GRAYSCALE, x, y, &analysis);
        RECT b;
        analysis->GetAlphaTextureBounds(DWRITE_TEXTURE_TYPE.DWRITE_TEXTURE_ALIASED_1x1, &b);
        bounds = b;
        return analysis;
    }

    // Composites the first `count` layers over a cleared canvas, releasing
    // each; a plain glyph is one white layer, which leaves (c, c, c, c).
    private bool Draw(int count, RECT canvas, bool isColor, out GlyphBitmap bitmap)
    {
        int w = canvas.right - canvas.left, h = canvas.bottom - canvas.top;
        bool drawn = count > 0 && w > 0 && h > 0 && w <= MaxSide && h <= MaxSide;
        if (drawn && pixels.Length < w * h * 4)
        {
            pixels = new byte[w * h * 4];
            coverage = new byte[w * h];
        }
        bitmap = drawn
            ? new GlyphBitmap { Width = w, Height = h, Left = canvas.left, Top = -canvas.top, IsColor = isColor, Pixels = pixels.AsSpan(0, w * h * 4) }
            : default;
        bitmap.Pixels.Clear();
        for (int i = 0; i < count; i++)
        {
            var analysis = (IDWriteGlyphRunAnalysis*)layers[i];
            if (drawn)
            {
                Composite(analysis, canvas, layerColors[i], bitmap.Pixels);
            }
            analysis->Release();
            layers[i] = 0;
        }
        return drawn;
    }

    private void Composite(IDWriteGlyphRunAnalysis* analysis, RECT canvas, DWRITE_COLOR_F tint, Span<byte> target)
    {
        RECT layer;
        analysis->GetAlphaTextureBounds(DWRITE_TEXTURE_TYPE.DWRITE_TEXTURE_ALIASED_1x1, &layer);
        int lw = layer.right - layer.left, lh = layer.bottom - layer.top;
        if (lw <= 0 || lh <= 0)
        {
            return;
        }
        fixed (byte* c = coverage)
        {
            analysis->CreateAlphaTexture(DWRITE_TEXTURE_TYPE.DWRITE_TEXTURE_ALIASED_1x1, &layer, c, (uint)(lw * lh));
        }
        int cw = canvas.right - canvas.left;
        for (int y = 0; y < lh; y++)
        {
            for (int x = 0; x < lw; x++)
            {
                float a = coverage[y * lw + x] / 255f * tint.a;
                if (a > 0)
                {
                    // Source over, premultiplied.
                    Span<byte> px = target.Slice(((layer.top - canvas.top + y) * cw + layer.left - canvas.left + x) * 4, 4);
                    px[0] = (byte)(tint.b * a * 255 + px[0] * (1 - a) + 0.5f);
                    px[1] = (byte)(tint.g * a * 255 + px[1] * (1 - a) + 0.5f);
                    px[2] = (byte)(tint.r * a * 255 + px[2] * (1 - a) + 0.5f);
                    px[3] = (byte)(a * 255 + px[3] * (1 - a) + 0.5f);
                }
            }
        }
    }

    public void Dispose()
    {
        for (int i = 0; i < faces.Length; i++)
        {
            var face = (IDWriteFontFace*)faces[i];
            Com.Release(ref face);
            faces[i] = 0;
        }
        Com.Release(ref factory);
    }
}
