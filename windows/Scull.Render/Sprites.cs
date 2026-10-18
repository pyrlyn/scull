using System.Numerics;
using Scull.Core;

namespace Scull.Render;

/// <summary>
/// Characters drawn from geometry instead of the font: box drawing, block
/// elements, Powerline separators and the underline kinds, the shapes of
/// Sprites.swift. A font's own box glyphs rarely fill the cell exactly, so
/// lines break between rows; drawn to the cell's own size they always meet.
/// </summary>
public static class Sprites
{
    /// <summary>The <see cref="GlyphKey.Face"/> of every sprite; no font list comes near it.</summary>
    public const ushort Face = ushort.MaxValue;

    /// <summary>Box drawing and block elements, then the Powerline separators.</summary>
    public static bool Covers(uint codePoint) => codePoint is (>= 0x2500 and <= 0x259F) or (>= 0xE0B0 and <= 0xE0BF);

    /// <summary>The sprite of a covered character.</summary>
    public static GlyphKey Key(uint codePoint) => new(Face, (ushort)codePoint);

    /// <summary>The sprite of an underline kind; every kind is below the first covered character.</summary>
    public static GlyphKey UnderlineKey(Underline kind) => new(Face, (byte)kind);
}

/// <summary>
/// Rasterises sprites as white coverage, cropped to their ink, into buffers it
/// reuses. Rectangles get their exact pixel area; slanted and curved shapes
/// are supersampled. Only an atlas miss draws, so none of this is per frame.
/// </summary>
internal sealed class SpriteCanvas
{
    // Arms of U+2500...U+257F as up, right, down, left: 0 none, 1 light,
    // 2 heavy, 3 double. Dashes, arcs and diagonals are drawn apart (0000).
    private static readonly byte[] Arms = """
        0101 0202 1010 2020 0000 0000 0000 0000 0000 0000 0000 0000 0110 0210 0120 0220
        0011 0012 0021 0022 1100 1200 2100 2200 1001 1002 2001 2002 1110 1210 2110 1120
        2120 2210 1220 2220 1011 1012 2011 1021 2021 2012 1022 2022 0111 0112 0211 0212
        0121 0122 0221 0222 1101 1102 1201 1202 2101 2102 2201 2202 1111 1112 1211 1212
        2111 1121 2121 2112 2211 1122 1221 2212 1222 2122 2221 2222 0000 0000 0000 0000
        0303 3030 0310 0130 0330 0013 0031 0033 1300 3100 3300 1003 3001 3003 1310 3130
        3330 1013 3031 3033 0313 0131 0333 1303 3101 3303 1313 3131 3333 0000 0000 0000
        0000 0000 0000 0000 0001 1000 0100 0010 0002 2000 0200 0020 0201 1020 0102 2010
        """.Where(c => c is >= '0' and <= '3').Select(c => (byte)(c - '0')).ToArray();

    // Quadrants of U+2596...U+259F: 1 upper left, 2 upper right, 4 lower
    // left, 8 lower right.
    private static ReadOnlySpan<byte> Quadrants => [4, 8, 1, 13, 9, 7, 11, 2, 6, 14];

    // Samples per pixel side for shapes that are not rectangles.
    private const int Samples = 4;
    private const int EllipseSegments = 48, ArcSegments = 16;

    private float[] coverage = [];
    private byte[] pixels = [];
    private Vector2[] points = new Vector2[EllipseSegments + 1];
    private CellMetrics cell;
    private int w, h;
    private float t, cx, cy;

    /// <summary>Draws the sprite <paramref name="key"/> for cells of <paramref name="metrics"/>; false when it draws nothing.</summary>
    public bool Draw(GlyphKey key, CellMetrics metrics, out GlyphBitmap bitmap)
    {
        bitmap = default;
        if (key.Face != Sprites.Face)
        {
            return false;
        }
        cell = metrics;
        (w, h) = (Math.Max(1, metrics.Width), Math.Max(1, metrics.Height));
        t = Math.Max(1, metrics.Thickness);
        (cx, cy) = (w / 2, h / 2);
        if (coverage.Length < w * h)
        {
            coverage = new float[w * h];
            pixels = new byte[w * h * 4];
        }
        if (points.Length < w + 2)
        {
            points = new Vector2[w + 2];
        }
        coverage.AsSpan(0, w * h).Clear();
        Paint(key.Glyph);
        return Crop(out bitmap);
    }

    private void Paint(uint code)
    {
        switch (code)
        {
            case >= 0x2504 and <= 0x250B: Dashes(code - 0x2504, code < 0x2508 ? 3 : 4); break;
            case >= 0x254C and <= 0x254F: Dashes(code - 0x254C, 2); break;
            case >= 0x256D and <= 0x2570: Arc(code); break;
            case >= 0x2571 and <= 0x2573: Diagonals(code); break;
            case >= 0x2500 and <= 0x257F: Lines(Arms.AsSpan((int)(code - 0x2500) * 4, 4)); break;
            case 0x2580: Fill(0, 0, 1, 0.5f); break;
            case >= 0x2581 and <= 0x2588: Fill(0, 1 - (code - 0x2580) / 8f, 1, 1); break;
            case >= 0x2589 and <= 0x258F: Fill(0, 0, (0x2590 - code) / 8f, 1); break;
            case 0x2590: Fill(0.5f, 0, 1, 1); break;
            // Shades as even coverage, not a dither that beats against the
            // pixel grid.
            case >= 0x2591 and <= 0x2593: Fill(0, 0, 1, 1, (code - 0x2590) / 4f); break;
            case 0x2594: Fill(0, 0, 1, 1 / 8f); break;
            case 0x2595: Fill(7 / 8f, 0, 1, 1); break;
            case >= 0x2596 and <= 0x259F:
                int mask = Quadrants[(int)(code - 0x2596)];
                for (int quadrant = 0; quadrant < 4; quadrant++)
                {
                    if ((mask & 1 << quadrant) != 0)
                    {
                        float x = quadrant % 2 * 0.5f, y = quadrant / 2 * 0.5f;
                        Fill(x, y, x + 0.5f, y + 0.5f);
                    }
                }
                break;
            case >= 0xE0B0 and <= 0xE0BF: Powerline(code); break;
            default: Underlines((Underline)code); break;
        }
    }

    private static float Round(float v) => MathF.Round(v, MidpointRounding.AwayFromZero);

    // Never throws on an inverted range, which a tiny cell can give: the low end wins.
    private static float Within(float v, float low, float high) => Math.Max(low, Math.Min(v, high));

    /// <summary>A rectangle in fractions of the cell, its edges on whole pixels.</summary>
    private void Fill(float x0, float y0, float x1, float y1, float alpha = 1) =>
        Rect(Round(x0 * w), Round(y0 * h), Round(x1 * w), Round(y1 * h), alpha);

    /// <summary>A rectangle in pixels; a pixel it partly covers gets that share.</summary>
    private void Rect(float x0, float y0, float x1, float y1, float alpha = 1)
    {
        (x0, x1) = (Math.Max(0, Math.Min(x0, x1)), Math.Min(w, Math.Max(x0, x1)));
        (y0, y1) = (Math.Max(0, Math.Min(y0, y1)), Math.Min(h, Math.Max(y0, y1)));
        for (int y = (int)y0; y < y1; y++)
        {
            float down = Math.Min(y1, y + 1) - Math.Max(y0, y);
            for (int x = (int)x0; x < x1; x++)
            {
                Blend(y * w + x, alpha * down * (Math.Min(x1, x + 1) - Math.Max(x0, x)));
            }
        }
    }

    // Source over, so overlapping arms never pass full coverage.
    private void Blend(int i, float a)
    {
        if (a > 0)
        {
            coverage[i] = a + coverage[i] * (1 - a);
        }
    }

    /// <summary>
    /// A pixel-aligned bar of thickness <paramref name="th"/> centred
    /// <paramref name="offset"/> off the cell's middle, from
    /// <paramref name="start"/> past the centre (negative: short of it) to the
    /// edge in <paramref name="direction"/> (0 up, 1 right, 2 down, 3 left).
    /// </summary>
    private void Bar(int direction, float start, float offset, float th)
    {
        bool horizontal = direction % 2 == 1;
        float across = MathF.Floor((horizontal ? cy : cx) + offset - th / 2);
        float middle = horizontal ? cx : cy;
        (float from, float to) = direction is 1 or 2 ? (middle + start, horizontal ? w : h) : (0f, middle - start);
        if (horizontal)
        {
            Rect(from, across, to, across + th);
        }
        else
        {
            Rect(across, from, across + th, to);
        }
    }

    private void Lines(ReadOnlySpan<byte> arm)
    {
        float d = t;
        float widest = t;
        for (int dir = 0; dir < 4; dir++)
        {
            widest = Math.Max(widest, arm[dir] == 3 ? d * 2 + t : Weight(arm[dir]));
        }
        Span<byte> sides = stackalloc byte[2];
        for (int dir = 0; dir < 4; dir++)
        {
            if (arm[dir] == 0)
            {
                continue;
            }
            // The perpendicular arms, on the side of a negative offset first.
            byte opposite = arm[(dir + 2) % 4];
            (sides[0], sides[1]) = dir % 2 == 1 ? (arm[0], arm[2]) : (arm[3], arm[1]);
            if (arm[dir] == 3)
            {
                // Each of the two lines stops at a double line crossing it,
                // runs on into a continuing arm, meets a single line at the
                // centre, and otherwise turns the outer corner.
                for (int i = 0; i < 2; i++)
                {
                    byte side = sides[i], other = sides[1 - i];
                    float start = side == 3 ? d
                        : opposite != 0 ? -d - t / 2
                        : side is 1 or 2 || other is 1 or 2 ? -t / 2 : -d - t / 2;
                    Bar(dir, start, i == 0 ? -d : d, t);
                }
            }
            else
            {
                // A stem off a double line stops at its near line, or at the
                // far one where the double line turns a corner.
                float start = opposite == 0 && (sides[0] == 3 || sides[1] == 3)
                    ? (sides[0] != 0 && sides[1] != 0 ? d : -d) : -widest / 2;
                Bar(dir, start, 0, Weight(arm[dir]));
            }
        }
    }

    private float Weight(byte arm) => arm == 2 ? t * 2 : t;

    private void Dashes(uint index, int segments)
    {
        bool vertical = index % 4 >= 2;
        float th = index % 2 == 1 ? t * 2 : t;
        float slot = (float)(vertical ? h : w) / segments;
        float gap = Math.Max(1, Round(slot / 3));
        float across = MathF.Floor((vertical ? cx : cy) - th / 2);
        for (int i = 0; i < segments; i++)
        {
            float from = Round(i * slot + gap / 2), to = Round((i + 1) * slot - gap / 2);
            if (vertical)
            {
                Rect(across, from, across + th, to);
            }
            else
            {
                Rect(from, across, to, across + th);
            }
        }
    }

    private void Arc(uint code)
    {
        // The stroke's centre line sits where a light bar's centre does.
        float px = MathF.Floor(cx - t / 2) + t / 2, py = MathF.Floor(cy - t / 2) + t / 2;
        float sy = code is 0x256D or 0x256E ? 1 : -1, sx = code is 0x256D or 0x2570 ? 1 : -1;
        float r = Math.Max(1, Math.Min(w, h) / 2f - t);
        // From the vertical edge to the corner's arc, round it, then on to the
        // horizontal edge.
        points[0] = new Vector2(px, sy > 0 ? h : 0);
        var centre = new Vector2(px + sx * r, py + sy * r);
        for (int i = 0; i <= ArcSegments; i++)
        {
            float angle = MathF.PI / 2 * i / ArcSegments;
            points[1 + i] = centre + new Vector2(-sx * r * MathF.Cos(angle), -sy * r * MathF.Sin(angle));
        }
        points[ArcSegments + 2] = new Vector2(sx > 0 ? w : 0, py);
        Stroke(ArcSegments + 3);
    }

    private void Diagonals(uint code)
    {
        if (code != 0x2572)
        {
            Segment(new Vector2(w, 0), new Vector2(0, h));
        }
        if (code != 0x2571)
        {
            Segment(new Vector2(0, 0), new Vector2(w, h));
        }
    }

    private void Segment(Vector2 from, Vector2 to)
    {
        (points[0], points[1]) = (from, to);
        Stroke(2);
    }

    /// <summary>
    /// Powerline separators: even codes are solid, odd ones their outline. The
    /// solid shapes reach the cell edges, so they meet the background of the
    /// next segment without a seam.
    /// </summary>
    private void Powerline(uint code)
    {
        bool solid = code % 2 == 0;
        switch (code)
        {
            case 0xE0B0 or 0xE0B1: Shape(solid, (0, 0), (1, 0.5f), (0, 1)); break;
            case 0xE0B2 or 0xE0B3: Shape(solid, (1, 0), (0, 0.5f), (1, 1)); break;
            case >= 0xE0B4 and <= 0xE0B7:
                // Half an ellipse bulging away from the edge it stands on; the
                // outline is inset so the whole stroke stays in the cell.
                float inset = solid ? 0 : t / 2;
                float left = code < 0xE0B6 ? -w : inset, width = w * 2 - inset, height = h - inset * 2;
                var centre = new Vector2(left + width / 2, inset + height / 2);
                for (int i = 0; i <= EllipseSegments; i++)
                {
                    float angle = MathF.Tau * i / EllipseSegments;
                    points[i] = centre + new Vector2(width / 2 * MathF.Cos(angle), height / 2 * MathF.Sin(angle));
                }
                if (solid)
                {
                    Polygon(EllipseSegments);
                }
                else
                {
                    Stroke(EllipseSegments + 1);
                }
                break;
            case 0xE0B8: Shape(true, (0, 0), (1, 1), (0, 1)); break;
            case 0xE0BA: Shape(true, (1, 0), (1, 1), (0, 1)); break;
            case 0xE0BC: Shape(true, (0, 0), (1, 0), (0, 1)); break;
            case 0xE0BE: Shape(true, (0, 0), (1, 0), (1, 1)); break;
            // The outlines of the corner triangles are their hypotenuses.
            case 0xE0B9 or 0xE0BF: Segment(new Vector2(0, 0), new Vector2(w, h)); break;
            default: Segment(new Vector2(w, 0), new Vector2(0, h)); break;
        }
    }

    /// <summary>Three points in fractions of the cell: a filled triangle, or the open line through them.</summary>
    private void Shape(bool solid, (float X, float Y) a, (float X, float Y) b, (float X, float Y) c)
    {
        points[0] = new Vector2(a.X * w, a.Y * h);
        points[1] = new Vector2(b.X * w, b.Y * h);
        points[2] = new Vector2(c.X * w, c.Y * h);
        if (solid)
        {
            Polygon(3);
        }
        else
        {
            Stroke(3);
        }
    }

    /// <summary>Fills the polygon of the first <paramref name="count"/> points, even-odd.</summary>
    private void Polygon(int count)
    {
        ReadOnlySpan<Vector2> p = points.AsSpan(0, count);
        Supersample(p, 0, static (p, s, _) =>
        {
            bool inside = false;
            for (int i = 0, j = p.Length - 1; i < p.Length; j = i++)
            {
                if ((p[i].Y > s.Y) != (p[j].Y > s.Y) && s.X < (p[j].X - p[i].X) * (s.Y - p[i].Y) / (p[j].Y - p[i].Y) + p[i].X)
                {
                    inside = !inside;
                }
            }
            return inside;
        });
    }

    /// <summary>Strokes the polyline through the first <paramref name="count"/> points, <c>t</c> wide.</summary>
    private void Stroke(int count)
    {
        ReadOnlySpan<Vector2> p = points.AsSpan(0, count);
        Supersample(p, t / 2, static (p, s, half) =>
        {
            for (int i = 0; i + 1 < p.Length; i++)
            {
                Vector2 along = p[i + 1] - p[i];
                float length = along.LengthSquared();
                float k = length > 0 ? Math.Clamp(Vector2.Dot(s - p[i], along) / length, 0, 1) : 0;
                if (Vector2.DistanceSquared(s, p[i] + along * k) <= half * half)
                {
                    return true;
                }
            }
            return false;
        });
    }

    private delegate bool Hit(ReadOnlySpan<Vector2> points, Vector2 sample, float half);

    /// <summary>Covers each pixel near the points by the share of its samples <paramref name="hit"/> takes.</summary>
    private void Supersample(ReadOnlySpan<Vector2> p, float half, Hit hit)
    {
        float x0 = w, y0 = h, x1 = 0, y1 = 0;
        foreach (Vector2 v in p)
        {
            (x0, y0, x1, y1) = (Math.Min(x0, v.X), Math.Min(y0, v.Y), Math.Max(x1, v.X), Math.Max(y1, v.Y));
        }
        int left = (int)Within(MathF.Floor(x0 - half), 0, w), top = (int)Within(MathF.Floor(y0 - half), 0, h);
        int right = (int)Within(MathF.Ceiling(x1 + half), 0, w), bottom = (int)Within(MathF.Ceiling(y1 + half), 0, h);
        for (int y = top; y < bottom; y++)
        {
            for (int x = left; x < right; x++)
            {
                int hits = 0;
                for (int sy = 0; sy < Samples; sy++)
                {
                    for (int sx = 0; sx < Samples; sx++)
                    {
                        hits += hit(p, new Vector2(x + (sx + 0.5f) / Samples, y + (sy + 0.5f) / Samples), half) ? 1 : 0;
                    }
                }
                Blend(y * w + x, hits / (float)(Samples * Samples));
            }
        }
    }

    private void Underlines(Underline kind)
    {
        // CellMetrics.Underline is the line's top row, where the straight
        // underline of T17.2 drew it.
        float u = Within(cell.Underline, 0, h - t);
        switch (kind)
        {
            case Underline.DoubleLine:
                float top = Within(u - t, 0, h - t * 3);
                Rect(0, top, w, top + t);
                Rect(0, top + t * 2, w, top + t * 3);
                break;
            case Underline.Curly:
                // One period per cell, so neighbouring cells join.
                float amplitude = t, middle = Within(u + t / 2, amplitude + t / 2, h - amplitude - t / 2);
                for (int x = 0; x <= w; x++)
                {
                    points[x] = new Vector2(x, middle - amplitude * MathF.Sin(MathF.Tau * x / w));
                }
                Stroke(w + 1);
                break;
            case Underline.Dotted:
                for (float x = 0; x < w; x += t * 2)
                {
                    Rect(x, u, x + t, u + t);
                }
                break;
            case Underline.Dashed:
                float from = Round(w * 0.15f);
                Rect(from, u, from + Round(w * 0.7f), u + t);
                break;
            default:
                Rect(0, u, w, u + t);
                break;
        }
    }

    private static byte Byte(float a) => (byte)(Math.Clamp(a, 0, 1) * 255 + 0.5f);

    /// <summary>The covered part of the canvas as premultiplied white, placed against the cell's baseline.</summary>
    private bool Crop(out GlyphBitmap bitmap)
    {
        int x0 = w, y0 = h, x1 = -1, y1 = -1;
        for (int y = 0; y < h; y++)
        {
            for (int x = 0; x < w; x++)
            {
                if (Byte(coverage[y * w + x]) != 0)
                {
                    (x0, y0, x1, y1) = (Math.Min(x0, x), Math.Min(y0, y), Math.Max(x1, x), Math.Max(y1, y));
                }
            }
        }
        if (x1 < 0)
        {
            bitmap = default;
            return false;
        }
        int bw = x1 - x0 + 1, bh = y1 - y0 + 1;
        Span<byte> target = pixels.AsSpan(0, bw * bh * 4);
        for (int y = 0; y < bh; y++)
        {
            for (int x = 0; x < bw; x++)
            {
                target.Slice((y * bw + x) * 4, 4).Fill(Byte(coverage[(y0 + y) * w + x0 + x]));
            }
        }
        bitmap = new GlyphBitmap { Width = bw, Height = bh, Left = x0, Top = cell.Baseline - y0, IsColor = false, Pixels = target };
        return true;
    }
}
