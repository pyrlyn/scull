namespace Scull.Render;

/// <summary>A glyph of one of the renderer's faces.</summary>
public readonly record struct GlyphKey(ushort Face, ushort Glyph);

/// <summary>Where a glyph's texels sit in the atlas and how they sit against the pen.</summary>
public readonly record struct AtlasEntry(ushort X, ushort Y, ushort Width, ushort Height, short Left, short Top, bool IsColor, ushort Shelf)
{
    /// <summary>Nothing to draw (a space); remembered so it is not rasterised again.</summary>
    public bool IsEmpty => Width == 0;
}

/// <summary>
/// The atlas's bookkeeping, without the texture: glyphs packed in shelves of
/// near-equal height, the same design as GlyphAtlas.swift. A terminal shows a
/// few hundred distinct glyphs at a time, so shelves pack it well, and a whole
/// shelf is the unit of eviction: cheap to track, no free-rectangle lists.
/// </summary>
public sealed class AtlasPacker
{
    private struct Shelf
    {
        public int Y, Height, X;
        public ulong LastUsed;
        public List<GlyphKey> Keys;
    }

    private readonly List<Shelf> shelves = [];
    private readonly Dictionary<GlyphKey, AtlasEntry> entries = [];
    private readonly int maxSize;
    private readonly ulong framesInFlight;
    private int nextY;

    /// <param name="framesInFlight">
    /// Frames the GPU may still be reading after the one being built. 0 suits
    /// Direct3D 11: the immediate context orders a texture update after every
    /// draw issued before it, so only shelves this frame drew are off limits.
    /// </param>
    public AtlasPacker(int size = 1024, int maxSize = 8192, ulong framesInFlight = 0)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(size, 16);
        Size = size;
        this.maxSize = Math.Max(size, maxSize);
        this.framesInFlight = framesInFlight;
    }

    /// <summary>The side of the square texture the entries index.</summary>
    public int Size { get; private set; }

    /// <summary>
    /// Bumped when the atlas starts over at a larger size: every entry handed
    /// out before is gone, so rows built with them must be rebuilt.
    /// </summary>
    public int Generation { get; private set; }

    public int Count => entries.Count;

    /// <summary>The entry for <paramref name="key"/>, its shelf marked as drawn in <paramref name="frame"/>.</summary>
    public bool TryGet(GlyphKey key, ulong frame, out AtlasEntry entry)
    {
        if (!entries.TryGetValue(key, out entry))
        {
            return false;
        }
        if (!entry.IsEmpty)
        {
            Touch(entry.Shelf, frame);
        }
        return true;
    }

    /// <summary>Remembers that <paramref name="key"/> draws nothing.</summary>
    public AtlasEntry AddEmpty(GlyphKey key) => entries[key] = default;

    /// <summary>
    /// Room for a <paramref name="width"/> x <paramref name="height"/> bitmap,
    /// evicting or growing as needed; null when it fits nowhere even at the
    /// largest size, and the caller skips the glyph.
    /// </summary>
    public AtlasEntry? Place(GlyphKey key, int width, int height, short left, short top, bool isColor, ulong frame)
    {
        // A texel of gap so linear sampling, should it ever be used, never bleeds.
        int w = width + 1;
        // Rounded so glyphs of nearly the same height share shelves.
        int h = (height + 1 + 3) / 4 * 4;
        while (w <= Size && h <= Size)
        {
            int index = ShelfWithRoom(w, h);
            if (index < 0)
            {
                index = NewShelf(h, frame);
            }
            if (index < 0)
            {
                index = Evict(h, frame);
            }
            if (index >= 0)
            {
                Shelf shelf = shelves[index];
                var entry = new AtlasEntry((ushort)shelf.X, (ushort)shelf.Y, (ushort)width, (ushort)height, left, top, isColor, (ushort)index);
                shelf.X += w;
                shelf.LastUsed = frame;
                shelf.Keys.Add(key);
                shelves[index] = shelf;
                entries[key] = entry;
                return entry;
            }
            if (!Grow())
            {
                break;
            }
        }
        return null;
    }

    /// <summary>Marks a shelf as drawn in <paramref name="frame"/>; rows reused from an earlier frame keep their glyphs this way.</summary>
    public void Touch(ushort shelf, ulong frame)
    {
        if (shelf < shelves.Count && shelves[shelf].LastUsed < frame)
        {
            Shelf s = shelves[shelf];
            s.LastUsed = frame;
            shelves[shelf] = s;
        }
    }

    private int ShelfWithRoom(int width, int height)
    {
        for (int i = 0; i < shelves.Count; i++)
        {
            Shelf s = shelves[i];
            if (s.Height >= height && s.Height <= height + height / 4 + 4 && s.X + width <= Size)
            {
                return i;
            }
        }
        return -1;
    }

    private int NewShelf(int height, ulong frame)
    {
        if (nextY + height > Size || shelves.Count >= ushort.MaxValue)
        {
            return -1;
        }
        shelves.Add(new Shelf { Y = nextY, Height = height, LastUsed = frame, Keys = [] });
        nextY += height;
        return shelves.Count - 1;
    }

    /// <summary>Empties the least recently drawn shelf tall enough for <paramref name="height"/>.</summary>
    private int Evict(int height, ulong frame)
    {
        int best = -1;
        for (int i = 0; i < shelves.Count; i++)
        {
            Shelf s = shelves[i];
            if (s.Height >= height && s.LastUsed + framesInFlight < frame && (best < 0 || s.LastUsed < shelves[best].LastUsed))
            {
                best = i;
            }
        }
        if (best >= 0)
        {
            Shelf s = shelves[best];
            foreach (GlyphKey key in s.Keys)
            {
                entries.Remove(key);
            }
            s.Keys.Clear();
            s.X = 0;
            shelves[best] = s;
        }
        return best;
    }

    private bool Grow()
    {
        if (Size * 2 > maxSize)
        {
            return false;
        }
        Size *= 2;
        shelves.Clear();
        entries.Clear();
        nextY = 0;
        Generation++;
        return true;
    }
}
