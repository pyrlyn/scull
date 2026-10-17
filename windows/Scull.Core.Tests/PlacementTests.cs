using System.Text;

namespace Scull.Core.Tests;

/// <summary>Kitty graphics images fed to the real core and read back from the frame view.</summary>
[TestClass]
public sealed class PlacementTests
{
    // 2 x 2 opaque red pixels, RGBA, base64.
    private const string Red = "/wAA//8AAP//AAD//wAA/w==";

    private static Terminal Sized()
    {
        var terminal = new Terminal(4, 2);
        // The core sizes placements in cells from the cell's pixel size.
        terminal.Resize(4, 2, 64, 64);
        return terminal;
    }

    private static void Feed(Terminal terminal, string text)
    {
        terminal.Feed(Encoding.UTF8.GetBytes(text));
        terminal.Update();
    }

    [TestMethod]
    public void AnImageIsPlacedWithItsPixels()
    {
        using Terminal terminal = Sized();
        Feed(terminal, $"\e_Gf=32,s=2,v=2,a=T,i=1,c=1,r=1;{Red}\e\\");

        Placement p = terminal.View.Placements.ToArray().Single();
        Assert.AreEqual((0, 0u, 1u, 1u), (p.Row, p.Col, p.Cols, p.Rows));
        Assert.AreEqual((0u, 0u, 2u, 2u), (p.SourceX, p.SourceY, p.SourceWidth, p.SourceHeight));
        Assert.AreEqual(0, p.Z);
        Assert.AreNotEqual(0, p.Image);

        byte[] pixels = p.Pixels(out int width, out int height, out int stride).ToArray();
        Assert.AreEqual((2, 2), (width, height));
        Assert.IsGreaterThanOrEqualTo(8, stride);
        Assert.AreEqual("FF0000FF", Convert.ToHexString(pixels, 0, 4));
    }

    [TestMethod]
    public void PlacementsComeLowestZFirst()
    {
        using Terminal terminal = Sized();
        Feed(terminal, $"\e_Gf=32,s=2,v=2,a=T,i=1,c=1,r=1;{Red}\e\\");
        Feed(terminal, "\e[1;2H\e_Ga=p,i=1,c=1,r=1,z=-1,C=1\e\\");

        Placement[] placements = terminal.View.Placements.ToArray();
        Assert.AreEqual("-1@1,0@0", string.Join(",", placements.Select(p => $"{p.Z}@{p.Col}")));
        Assert.AreEqual(placements[0].Image, placements[1].Image, "one image, placed twice");
    }

    [TestMethod]
    public void ADeletedImageLeavesTheView()
    {
        using Terminal terminal = Sized();
        Feed(terminal, $"\e_Gf=32,s=2,v=2,a=T,i=1,c=1,r=1;{Red}\e\\");
        Feed(terminal, "\e_Ga=d,d=A\e\\");

        Assert.AreEqual(0, terminal.View.Placements.Length);
    }

    [TestMethod]
    public void NoImageHasNoPixels()
    {
        Assert.IsTrue(default(Placement).Pixels(out int width, out int height, out int stride).IsEmpty);
        Assert.AreEqual((0, 0, 0), (width, height, stride));
    }
}
