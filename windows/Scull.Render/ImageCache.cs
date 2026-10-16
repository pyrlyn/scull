namespace Scull.Render;

/// <summary>
/// Textures by image and generation, as ImageLayer.swift keeps them: a frame
/// keeps what it places and <see cref="EndFrame"/> releases the rest, since
/// the frame lists every image in the viewport. Two tables swapped each frame,
/// so a frame of images seen before allocates nothing.
/// </summary>
internal sealed class ImageCache<T>(Action<T> release)
{
    private Dictionary<(nint Image, ulong Generation), T> live = [], kept = [];

    public int Count => live.Count + kept.Count;

    /// <summary>The texture of an image placed earlier, kept for this frame.</summary>
    public bool TryGet(nint image, ulong generation, out T value)
    {
        if (kept.TryGetValue((image, generation), out value!))
        {
            return true;
        }
        if (!live.Remove((image, generation), out value!))
        {
            return false;
        }
        kept[(image, generation)] = value;
        return true;
    }

    /// <summary>A texture made this frame.</summary>
    public void Add(nint image, ulong generation, T value) => kept[(image, generation)] = value;

    /// <summary>Releases every texture this frame did not place.</summary>
    public void EndFrame()
    {
        foreach (T value in live.Values)
        {
            release(value);
        }
        live.Clear();
        (live, kept) = (kept, live);
    }

    public void Clear()
    {
        EndFrame();
        EndFrame();
    }
}
