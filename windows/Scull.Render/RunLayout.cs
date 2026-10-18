using System.Buffers;
using System.Globalization;
using System.Text;
using Windows.Win32.Graphics.DirectWrite;

namespace Scull.Render;

/// <summary>One glyph of a shaped run, placed against the run's first column.</summary>
/// <param name="Column">Column of the glyph's cluster, counted from the run's first column.</param>
/// <param name="Dx">Pixels right of the cluster's left edge.</param>
/// <param name="Dy">Pixels up from the baseline.</param>
public readonly record struct ShapedGlyph(GlyphKey Key, ushort Column, float Dx, float Dy);

/// <summary>
/// Puts shaped glyphs on the grid, as RunShaper.swift does. A fallback
/// font's advances are its own, but only the core decides columns: every
/// glyph is pinned to the column of its grapheme and keeps just its offset
/// inside it, so nothing drifts however wide the fallback is. Plain logic over
/// DirectWrite's outputs, so it is tested on every host.
/// </summary>
internal sealed class RunLayout
{
    private int[] column = new int[64], start = new int[64];
    // Units whose font glyphs are dropped: sprites, and spaces, which have no ink.
    private bool[] drop = new bool[64];
    private float[] pens = new float[64];
    private int units;

    /// <summary>The run's glyphs so far: sprites from <see cref="Begin"/>, then each <see cref="Place"/>.</summary>
    public List<ShapedGlyph> Glyphs { get; } = [];

    /// <summary>
    /// Starts a run of <paramref name="text"/>, each grapheme taking
    /// <paramref name="width"/> columns as the core's cells do. A grapheme
    /// that is one sprite character goes to <see cref="Sprites"/> here, and
    /// the font's glyphs for it are dropped later.
    /// </summary>
    public void Begin(ReadOnlySpan<char> text, int width)
    {
        if (column.Length < text.Length)
        {
            (column, start, drop) = (new int[text.Length], new int[text.Length], new bool[text.Length]);
        }
        Glyphs.Clear();
        units = text.Length;
        width = Math.Max(1, width);
        for (int offset = 0, index = 0; offset < text.Length; index++)
        {
            int length = Math.Max(1, StringInfo.GetNextTextElementLength(text[offset..]));
            bool isSprite = Rune.DecodeFromUtf16(text[offset..], out Rune rune, out int used) == OperationStatus.Done
                && used == length && Sprites.Covers((uint)rune.Value);
            int col = Math.Min(index * width, ushort.MaxValue);
            for (int unit = offset; unit < offset + length && unit < text.Length; unit++)
            {
                (column[unit], start[unit], drop[unit]) = (col, offset, isSprite || (length == 1 && text[offset] == ' '));
            }
            if (isSprite)
            {
                Glyphs.Add(new ShapedGlyph(Sprites.Key((uint)rune.Value), (ushort)col, 0, 0));
            }
            offset += length;
        }
    }

    /// <summary>
    /// Adds one shaped piece that starts at unit <paramref name="first"/> of
    /// the run: DirectWrite's cluster map (one entry per unit of the piece,
    /// the first glyph of its cluster), glyph ids, advances and offsets, left
    /// to right.
    /// </summary>
    public void Place(int first, ushort face, ReadOnlySpan<ushort> clusterMap, ReadOnlySpan<ushort> glyphs,
        ReadOnlySpan<float> advances, ReadOnlySpan<DWRITE_GLYPH_OFFSET> offsets)
    {
        int count = Math.Min(glyphs.Length, Math.Min(advances.Length, offsets.Length));
        if (pens.Length < count)
        {
            pens = new float[count];
        }
        float pen = 0;
        for (int g = 0; g < count; g++)
        {
            pens[g] = pen + offsets[g].advanceOffset;
            pen += advances[g];
        }
        int grapheme = -1;
        float origin = 0;
        for (int c = 0; c < clusterMap.Length && first + c < units;)
        {
            int unit = first + c, from = clusterMap[c];
            while (c < clusterMap.Length && clusterMap[c] == from)
            {
                c++;
            }
            // A malformed map gives an empty range rather than a read past the glyphs.
            int to = Math.Min(count, Math.Max(from, c < clusterMap.Length ? clusterMap[c] : count));
            if (drop[unit])
            {
                continue;
            }
            for (int g = from; g < to; g++)
            {
                // A grapheme's origin is the pen of its first glyph; marks
                // keep their offset from it.
                if (start[unit] != grapheme)
                {
                    (grapheme, origin) = (start[unit], pens[g]);
                }
                Glyphs.Add(new ShapedGlyph(new GlyphKey(face, glyphs[g]), (ushort)column[unit], pens[g] - origin, offsets[g].ascenderOffset));
            }
        }
    }
}
