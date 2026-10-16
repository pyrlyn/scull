using System.Runtime.Versioning;
using System.Text;
using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>Renderers of one window sharing glyph atlases, drawn on WARP and read back.</summary>
[TestClass]
[OSCondition(OperatingSystems.Windows)]
[SupportedOSPlatform("windows8.1")]
public sealed class GlyphAtlasTests
{
    /// <summary>A pane: a terminal, its renderer on a shared pool and a target of its own.</summary>
    private sealed class Pane : IDisposable
    {
        public readonly Terminal Terminal = new(8, 1);
        public readonly Renderer Renderer;
        private readonly OffscreenTarget target;

        public Pane(RenderDevice device, GlyphAtlases atlases, float size = 16)
        {
            Renderer = new Renderer(device, atlases, "Consolas", size);
            target = new OffscreenTarget(device, 8 * Renderer.Metrics.Width, Renderer.Metrics.Height);
            Terminal.Feed("\e[?25l"u8);
        }

        public byte[] Draw(string text = "")
        {
            if (text.Length > 0)
            {
                Terminal.Feed(Encoding.UTF8.GetBytes($"\r\e[K{text}"));
            }
            Terminal.Update();
            Renderer.Absorb(Terminal.View);
            Renderer.Render(Terminal.View, target);
            return target.ReadPixels();
        }

        public void Dispose()
        {
            target.Dispose();
            Renderer.Dispose();
            Terminal.Dispose();
        }
    }

    [TestMethod]
    public void TwoRenderersOnOneAtlasRasteriseAGlyphOnce()
    {
        using var device = new RenderDevice(warp: true);
        var atlases = new GlyphAtlases(device);
        using var left = new Pane(device, atlases);
        using var right = new Pane(device, atlases);
        Assert.AreSame(left.Renderer.Atlas, right.Renderer.Atlas);
        Assert.AreEqual(1, atlases.Count);

        byte[] first = left.Draw("Mg");
        int drawn = left.Renderer.Atlas.Rasterized;
        Assert.IsGreaterThan(0, drawn);
        byte[] second = right.Draw("Mg");
        Assert.AreEqual(drawn, right.Renderer.Atlas.Rasterized, "the second pane found the glyphs drawn");
        CollectionAssert.AreEqual(first, second);
    }

    [TestMethod]
    public void AnotherSizeGetsItsOwnAtlas()
    {
        using var device = new RenderDevice(warp: true);
        var atlases = new GlyphAtlases(device);
        using var normal = new Pane(device, atlases);
        using var zoomed = new Pane(device, atlases, size: 20);
        Assert.AreNotSame(normal.Renderer.Atlas, zoomed.Renderer.Atlas);
        Assert.AreEqual(2, atlases.Count);

        using var other = new Pane(device, atlases);
        other.Renderer.SetFont("Consolas", 20);
        Assert.AreSame(zoomed.Renderer.Atlas, other.Renderer.Atlas, "zoomed to the same size, it joins that atlas");
        Assert.AreEqual(1, normal.Renderer.Atlas.Users);
        Assert.AreEqual(2, zoomed.Renderer.Atlas.Users);
    }

    [TestMethod]
    public void AnAtlasLivesUntilItsLastUserGoes()
    {
        using var device = new RenderDevice(warp: true);
        var atlases = new GlyphAtlases(device);
        var first = new Pane(device, atlases);
        using var second = new Pane(device, atlases);
        GlyphAtlas atlas = first.Renderer.Atlas;
        byte[] before = second.Draw("ab");
        first.Draw("cd");

        first.Dispose();
        Assert.AreEqual(1, atlas.Users);
        Assert.AreEqual(1, atlases.Count);
        second.Draw("cd");
        CollectionAssert.AreEqual(before, second.Draw("ab"), "the other pane still draws from it");

        second.Dispose();
        Assert.AreEqual(0, atlases.Count, "the last user freed it");
    }

    [TestMethod]
    public void EvictionsForOnePaneKeepTheGlyphsANeighbourDraws()
    {
        using var device = new RenderDevice(warp: true);
        // Room for a few shelves only, so the busy pane has to evict.
        var atlases = new GlyphAtlases(device, size: 64, maxSize: 64);
        using var quiet = new Pane(device, atlases);
        using var busy = new Pane(device, atlases);
        byte[] before = quiet.Draw("AB");

        foreach (string text in new[] { "CDEFGHIJ", "KLMNOPQR", "STUVWXYZ", "cdefghij", "klmnopqr" })
        {
            busy.Draw(text);
        }
        GlyphAtlas atlas = quiet.Renderer.Atlas;
        Assert.IsGreaterThan(atlas.Count, atlas.Rasterized, "glyphs were evicted");
        // Nothing changed in the quiet pane, so its rows are not rebuilt: they draw what the atlas still holds.
        CollectionAssert.AreEqual(before, quiet.Draw());
    }
}
