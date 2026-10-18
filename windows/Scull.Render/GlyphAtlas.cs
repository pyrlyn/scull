using System.Runtime.Versioning;
using Windows.Win32.Graphics.Direct3D11;

namespace Scull.Render;

/// <summary>
/// One font at one pixel size and the glyphs drawn with it: the faces, the
/// shaped runs, the shelf-packed texture. The renderers of one window's panes
/// share it when their fonts match, so a glyph is rasterised and uploaded once.
/// It lives while a renderer uses it; the last <see cref="Release"/> frees it.
/// </summary>
[SupportedOSPlatform("windows8.1")]
public sealed unsafe class GlyphAtlas
{
    private readonly RenderDevice device;
    private readonly GlyphAtlases pool;
    private readonly SpriteCanvas sprites = new();
    private readonly List<Renderer> users = [];
    private ID3D11Texture2D* texture;
    private ID3D11ShaderResourceView* view;
    private int textureSize;

    internal GlyphAtlas(RenderDevice device, GlyphAtlases pool, string family, float sizePixels, AtlasPacker packer)
    {
        (this.device, this.pool, Family, SizePixels, Packer) = (device, pool, family, sizePixels, packer);
        Fonts = new FontFaces(family, sizePixels);
        try
        {
            Shaper = new RunShaper(Fonts);
        }
        catch
        {
            Fonts.Dispose();
            throw;
        }
    }

    public string Family { get; }

    public float SizePixels { get; }

    /// <summary>Renderers using it.</summary>
    public int Users => users.Count;

    /// <summary>Distinct glyphs held, blanks included.</summary>
    public int Count => Packer.Count;

    /// <summary>Changes when the texture grew and every entry was dropped; rows built before point at nothing.</summary>
    public int Generation => Packer.Generation;

    /// <summary>Glyphs and sprites drawn into it so far, for tests of the sharing.</summary>
    internal int Rasterized { get; private set; }

    internal FontFaces Fonts { get; }

    internal RunShaper Shaper { get; }

    internal AtlasPacker Packer { get; }

    internal ID3D11ShaderResourceView* View => view;

    /// <summary>One counter for every user, so their shelves' ages compare.</summary>
    internal ulong Frame { get; private set; }

    internal void Acquire(Renderer user) => users.Add(user);

    /// <summary>Drops a user; the last one frees the faces and the texture.</summary>
    internal void Release(Renderer user)
    {
        if (!users.Remove(user) || users.Count > 0)
        {
            return;
        }
        pool.Forget(this);
        Com.Release(ref view);
        Com.Release(ref texture);
        Shaper.Dispose();
        Fonts.Dispose();
    }

    /// <summary>
    /// Starts a user's frame. Every user's rows keep drawing from their shelves
    /// without being rebuilt, so all of those survive the evictions this frame
    /// may cause, not only the drawing renderer's own.
    /// </summary>
    internal void BeginFrame()
    {
        Frame++;
        foreach (Renderer user in users)
        {
            foreach (HashSet<ushort> shelves in user.Shelves)
            {
                foreach (ushort shelf in shelves)
                {
                    Packer.Touch(shelf, Frame);
                }
            }
        }
    }

    /// <summary>The atlas entry for a glyph or sprite, drawn and uploaded on a miss; default when it fits nowhere.</summary>
    internal AtlasEntry Entry(GlyphKey key)
    {
        if (Packer.TryGet(key, Frame, out AtlasEntry entry))
        {
            return entry;
        }
        Rasterized++;
        bool drawn = key.Face == Sprites.Face ? sprites.Draw(key, Fonts.Metrics, out GlyphBitmap bitmap) : Fonts.Rasterize(key, out bitmap);
        if (!drawn)
        {
            return Packer.AddEmpty(key);
        }
        // Not remembered when it fits nowhere: a later eviction may make room.
        if (Packer.Place(key, bitmap.Width, bitmap.Height, (short)bitmap.Left, (short)bitmap.Top, bitmap.IsColor, Frame) is not AtlasEntry placed)
        {
            return default;
        }
        if (textureSize != Packer.Size)
        {
            // Growth dropped every entry, so the old texels are not needed.
            Com.Release(ref view);
            Com.Release(ref texture);
            textureSize = Packer.Size;
            texture = device.CreateTexture(textureSize, textureSize, D3D11_BIND_FLAG.D3D11_BIND_SHADER_RESOURCE);
            fixed (ID3D11ShaderResourceView** v = &view)
            {
                device.Device->CreateShaderResourceView((ID3D11Resource*)texture, null, v);
            }
        }
        var box = new D3D11_BOX { left = placed.X, top = placed.Y, right = (uint)(placed.X + placed.Width), bottom = (uint)(placed.Y + placed.Height), back = 1 };
        fixed (byte* pixels = bitmap.Pixels)
        {
            device.Context->UpdateSubresource((ID3D11Resource*)texture, 0, &box, pixels, (uint)(bitmap.Width * 4), 0);
        }
        return placed;
    }
}

/// <summary>
/// The glyph atlases of one device by font family and pixel size: a window
/// holds one and hands it to its panes' renderers, so panes whose fonts match
/// share an atlas and a zoomed pane gets its own.
/// </summary>
[SupportedOSPlatform("windows8.1")]
public sealed class GlyphAtlases
{
    private readonly Dictionary<(string Family, float Size), GlyphAtlas> live = [];
    private readonly RenderDevice device;
    private readonly int size, maxSize;

    public GlyphAtlases(RenderDevice device)
        : this(device, 1024, 8192)
    {
    }

    /// <summary>Atlases of a fixed texture size, so tests can fill one.</summary>
    internal GlyphAtlases(RenderDevice device, int size, int maxSize) => (this.device, this.size, this.maxSize) = (device, size, maxSize);

    /// <summary>Atlases in use.</summary>
    public int Count => live.Count;

    /// <summary>
    /// The live atlas for this font, or a new one; a missing family throws
    /// <see cref="ArgumentException"/>. The renderer that takes it acquires it.
    /// </summary>
    internal GlyphAtlas Get(string family, float sizePixels)
    {
        if (!live.TryGetValue((family, sizePixels), out GlyphAtlas? atlas))
        {
            atlas = new GlyphAtlas(device, this, family, sizePixels, new AtlasPacker(size, maxSize));
            live[(family, sizePixels)] = atlas;
        }
        return atlas;
    }

    internal void Forget(GlyphAtlas atlas) => live.Remove((atlas.Family, atlas.SizePixels));
}
