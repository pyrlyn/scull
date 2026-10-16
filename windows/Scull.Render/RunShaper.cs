using System.Runtime.Versioning;
using System.Text;
using Windows.Win32.Foundation;
using Windows.Win32.Graphics.DirectWrite;

namespace Scull.Render;

/// <summary>
/// Shapes a row's runs with DirectWrite, as RunShaper.swift does with Core
/// Text: script items, the system font fallback for characters the font
/// lacks, then glyphs and placements per piece, laid on the grid by
/// <see cref="RunLayout"/>. Shaped runs are cached by text, face and width, so
/// a frame of runs seen before shapes nothing and allocates nothing.
/// </summary>
[SupportedOSPlatform("windows8.1")]
internal sealed unsafe class RunShaper : IDisposable
{
    private static readonly HRESULT NotSufficientBuffer = (HRESULT)unchecked((int)0x8007007A);

    private readonly FontFaces fonts;
    private readonly TextAnalysis analysis = new();
    private readonly RunLayout layout = new();
    private readonly RunCache<ShapedGlyph[]> cache;
    private IDWriteTextAnalyzer* analyzer;
    private IDWriteFontFallback* fallback;
    // The key is the face and width as two chars, then the run's text.
    private char[] key = new char[256];
    private ushort[] clusterMap = new ushort[64], glyphs = new ushort[112];
    private DWRITE_SHAPING_TEXT_PROPERTIES[] textProps = new DWRITE_SHAPING_TEXT_PROPERTIES[64];
    private DWRITE_SHAPING_GLYPH_PROPERTIES[] glyphProps = new DWRITE_SHAPING_GLYPH_PROPERTIES[112];
    private float[] advances = new float[112];
    private DWRITE_GLYPH_OFFSET[] offsets = new DWRITE_GLYPH_OFFSET[112];

    public RunShaper(FontFaces fonts, int limit = 4096)
    {
        this.fonts = fonts;
        cache = new RunCache<ShapedGlyph[]>(limit);
        IDWriteTextAnalyzer* a;
        IDWriteFontFallback* f;
        fonts.Factory->CreateTextAnalyzer(&a);
        analyzer = a;
        fonts.Factory->GetSystemFontFallback(&f);
        fallback = f;
    }

    /// <summary>Runs held in the cache, for tests of its bound.</summary>
    public int Cached => cache.Count;

    /// <summary>
    /// The glyphs of a run's UTF-8 <paramref name="text"/> in style
    /// <paramref name="face"/> (0 regular, 1 bold, 2 italic, 3 both), each
    /// grapheme <paramref name="width"/> columns wide.
    /// </summary>
    public ShapedGlyph[] Shape(ReadOnlySpan<byte> text, int face, int width)
    {
        int length = Encoding.UTF8.GetMaxCharCount(text.Length) + 2;
        if (key.Length < length)
        {
            key = new char[length];
        }
        key[0] = (char)(face & 3);
        key[1] = (char)Math.Clamp(width, 1, 2);
        // Invalid bytes decode to U+FFFD, as the core's own text does.
        ReadOnlySpan<char> whole = key.AsSpan(0, 2 + Encoding.UTF8.GetChars(text, key.AsSpan(2)));
        if (cache.TryGet(whole, out ShapedGlyph[]? shaped))
        {
            return shaped;
        }
        shaped = Fresh(whole[2..], key[0], key[1]);
        cache.Add(whole, shaped);
        return shaped;
    }

    private ShapedGlyph[] Fresh(ReadOnlySpan<char> text, int face, int width)
    {
        layout.Begin(text, width);
        if (text.IsEmpty)
        {
            return [];
        }
        var weight = (face & 1) != 0 ? DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_BOLD : DWRITE_FONT_WEIGHT.DWRITE_FONT_WEIGHT_NORMAL;
        var style = (face & 2) != 0 ? DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_ITALIC : DWRITE_FONT_STYLE.DWRITE_FONT_STYLE_NORMAL;
        fixed (char* p = text)
        fixed (char* family = fonts.Family)
        {
            analysis.Use(p, text.Length);
            analysis.Analyse(analyzer);
            foreach ((uint start, uint count, DWRITE_SCRIPT_ANALYSIS script) in analysis.Scripts)
            {
                for (uint at = start, end = Math.Min(start + count, (uint)text.Length); at < end;)
                {
                    uint mapped = 0;
                    float scale = 1;
                    IDWriteFont* font = null;
                    fallback->MapCharacters(analysis.Source, at, end - at, fonts.Collection, family,
                        weight, style, DWRITE_FONT_STRETCH.DWRITE_FONT_STRETCH_NORMAL, &mapped, &font, &scale);
                    // No font at all draws the face's missing-glyph box, as a
                    // full fallback list does.
                    int id = face;
                    if (font != null)
                    {
                        id = fonts.Register(font, scale) is int known and >= 0 ? known : face;
                        font->Release();
                    }
                    mapped = Math.Clamp(mapped, 1, end - at);
                    Piece(p, at, mapped, (ushort)id, script);
                    at += mapped;
                }
            }
        }
        return [.. layout.Glyphs];
    }

    private void Piece(char* text, uint at, uint length, ushort face, DWRITE_SCRIPT_ANALYSIS script)
    {
        if (clusterMap.Length < length)
        {
            clusterMap = new ushort[length];
            textProps = new DWRITE_SHAPING_TEXT_PROPERTIES[length];
        }
        // DirectWrite's suggested first guess; it says when that is short.
        int max = Math.Max(glyphs.Length, (int)(3 * length / 2 + 16));
        uint count = 0;
        for (int attempt = 0; ; attempt++)
        {
            Grow(max);
            HRESULT result;
            fixed (ushort* map = clusterMap, ids = glyphs)
            fixed (DWRITE_SHAPING_TEXT_PROPERTIES* tp = textProps)
            fixed (DWRITE_SHAPING_GLYPH_PROPERTIES* gp = glyphProps)
            {
                result = analyzer->GetGlyphs(text + at, length, fonts[face], false, false, &script, default, null, null, null, 0,
                    (uint)max, map, tp, ids, gp, &count);
            }
            if (result == NotSufficientBuffer && attempt < 4)
            {
                max *= 2;
                continue;
            }
            if (result.Failed && script.script != 0 && attempt < 4)
            {
                // A script the face cannot shape is drawn unshaped rather than dropped.
                script = default;
                continue;
            }
            if (result.Failed)
            {
                return;
            }
            break;
        }
        fixed (ushort* map = clusterMap, ids = glyphs)
        fixed (DWRITE_SHAPING_TEXT_PROPERTIES* tp = textProps)
        fixed (DWRITE_SHAPING_GLYPH_PROPERTIES* gp = glyphProps)
        fixed (float* adv = advances)
        fixed (DWRITE_GLYPH_OFFSET* off = offsets)
        {
            analyzer->GetGlyphPlacements(text + at, map, tp, length, ids, gp, count, fonts[face], fonts.EmSize(face), false, false,
                &script, default, null, null, 0, adv, off);
        }
        layout.Place((int)at, face, clusterMap.AsSpan(0, (int)length), glyphs.AsSpan(0, (int)count),
            advances.AsSpan(0, (int)count), offsets.AsSpan(0, (int)count));
    }

    private void Grow(int max)
    {
        if (glyphs.Length < max)
        {
            glyphs = new ushort[max];
            glyphProps = new DWRITE_SHAPING_GLYPH_PROPERTIES[max];
            advances = new float[max];
            offsets = new DWRITE_GLYPH_OFFSET[max];
        }
    }

    public void Dispose()
    {
        Com.Release(ref fallback);
        Com.Release(ref analyzer);
        analysis.Dispose();
    }
}
