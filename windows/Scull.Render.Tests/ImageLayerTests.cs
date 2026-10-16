using System.Text;
using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>Image quads and the texture cache, with no device, so every host runs them.</summary>
[TestClass]
public sealed class ImageLayerTests
{
    private static readonly CellMetrics Cell = new(Width: 10, Height: 20, Baseline: 15, Underline: 17, Strike: 10, Thickness: 2);

    [TestMethod]
    public void APlacementCoversItsCellsMovedByItsOffset()
    {
        var box = new ImageBox(Row: 1, Col: 2, Cols: 3, Rows: 2, OffsetX: 4, OffsetY: 5, SourceX: 0, SourceY: 0, SourceWidth: 8, SourceHeight: 6);
        Assert.IsTrue(box.TryLayout(8, 6, Cell, out ImageQuad quad));
        Assert.AreEqual(new ImageQuad(24, 25, 30, 40, 0, 0, 8, 6), quad);
    }

    [TestMethod]
    public void APlacementAboveTheViewportStartsAboveIt()
    {
        var box = new ImageBox(-2, 0, 1, 3, 0, 0, 0, 0, 4, 4);
        Assert.IsTrue(box.TryLayout(4, 4, Cell, out ImageQuad quad));
        Assert.AreEqual(-40f, quad.Y);
    }

    [TestMethod]
    public void TheSourceRectangleStaysInsideThePixels()
    {
        var box = new ImageBox(0, 0, 1, 1, 0, 0, SourceX: 50, SourceY: 3, SourceWidth: 100, SourceHeight: 100);
        Assert.IsTrue(box.TryLayout(8, 6, Cell, out ImageQuad quad));
        Assert.AreEqual((7, 3, 1, 3), ((int)quad.U, (int)quad.V, (int)quad.SpanU, (int)quad.SpanV));
    }

    [TestMethod]
    public void AnEmptyBoxOrImageDrawsNothing()
    {
        Assert.IsFalse(new ImageBox(0, 0, 0, 1, 0, 0, 0, 0, 4, 4).TryLayout(4, 4, Cell, out _), "no columns");
        Assert.IsFalse(new ImageBox(0, 0, 1, 1, 0, 0, 0, 0, 0, 4).TryLayout(4, 4, Cell, out _), "no source width");
        Assert.IsFalse(new ImageBox(0, 0, 1, 1, 0, 0, 0, 0, 4, 4).TryLayout(0, 4, Cell, out _), "no pixels");
    }

    [TestMethod]
    public void ABoxComesFromTheFramesPlacement()
    {
        using var terminal = new Terminal(4, 2);
        terminal.Resize(4, 2, 40, 40);
        terminal.Feed(Encoding.UTF8.GetBytes("\e_Gf=32,s=2,v=2,a=T,i=1,c=2,r=1;/wAA//8AAP//AAD//wAA/w==\e\\"));
        terminal.Update();
        ImageBox box = ImageBox.Of(terminal.View.Placements[0]);
        Assert.AreEqual(new ImageBox(0, 0, 2, 1, 0, 0, 0, 0, 2, 2), box);
    }

    [TestMethod]
    public void ATextureLivesWhileAFramePlacesIt()
    {
        var released = new List<string>();
        var cache = new ImageCache<string>(released.Add);
        cache.Add(1, 0, "one");
        cache.Add(2, 0, "two");
        cache.EndFrame();

        Assert.IsTrue(cache.TryGet(1, 0, out string? one));
        Assert.AreEqual("one", one);
        cache.EndFrame();
        Assert.AreEqual("two", string.Join(",", released), "the image the frame no longer placed");
        Assert.AreEqual(1, cache.Count);
    }

    [TestMethod]
    public void ANewGenerationIsANewTexture()
    {
        var released = new List<string>();
        var cache = new ImageCache<string>(released.Add);
        cache.Add(1, 0, "old");
        cache.EndFrame();

        Assert.IsFalse(cache.TryGet(1, 1, out _), "the pixels may have changed");
        cache.Add(1, 1, "new");
        cache.EndFrame();
        Assert.AreEqual("old", string.Join(",", released));
    }

    [TestMethod]
    public void AFrameOfImagesSeenBeforeAllocatesNothing()
    {
        var cache = new ImageCache<string>(static _ => { });
        cache.Add(1, 0, "a");
        cache.EndFrame();

        long before = GC.GetAllocatedBytesForCurrentThread();
        for (int frame = 0; frame < 3; frame++)
        {
            cache.TryGet(1, 0, out _);
            cache.EndFrame();
        }
        Assert.AreEqual(0, GC.GetAllocatedBytesForCurrentThread() - before);
    }

    [TestMethod]
    public void ClearingReleasesEverything()
    {
        var released = new List<string>();
        var cache = new ImageCache<string>(released.Add);
        cache.Add(1, 0, "a");
        cache.EndFrame();
        cache.Add(2, 0, "b");
        cache.Clear();
        Assert.HasCount(2, released);
        Assert.AreEqual(0, cache.Count);
    }
}
