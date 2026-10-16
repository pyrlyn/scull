using System.Text;
using Scull.Core;

namespace Scull.Render.Tests;

/// <summary>Clusters split from a real terminal's frame, and the mark rules; no device, so every host runs them.</summary>
[TestClass]
public sealed class CellClustersTests
{
    private const ushort Cols = 8;

    /// <summary>The marks, as text, of each column of row <paramref name="row"/> after feeding <paramref name="text"/>.</summary>
    private static string[] Marks(string text, int row = 0, CellClusters? clusters = null)
    {
        using var terminal = new Terminal(Cols, 2);
        // Mode 2027: the core keeps whole grapheme clusters in one cell.
        terminal.Feed(Encoding.UTF8.GetBytes("\e[?2027h" + text));
        terminal.Update();
        FrameView view = terminal.View;
        clusters ??= new CellClusters();
        clusters.Load(view.Lines[row], view.Cells.Slice(row * Cols, Cols));
        return [.. Enumerable.Range(0, Cols).Select(col => string.Concat(clusters.At(col).ToArray().Select(char.ConvertFromUtf32)))];
    }

    [TestMethod]
    public void AClusterCellHoldsTheCodePointsAfterItsFirst()
    {
        string[] marks = Marks("e\u0301x");
        Assert.AreEqual("\u0301", marks[0]);
        Assert.AreEqual("", marks[1]);
    }

    [TestMethod]
    public void SeveralMarksStayInOrder()
    {
        string[] marks = Marks("ab\u0323\u0301c");
        Assert.AreEqual("", marks[0]);
        Assert.AreEqual("\u0323\u0301", marks[1]);
        Assert.AreEqual("", marks[2]);
    }

    [TestMethod]
    public void AWideClusterAndTheRunAfterItSplitByColumn()
    {
        string[] marks = Marks("\u4E2D\u0301a\u0300");
        Assert.AreEqual("\u0301", marks[0]);
        Assert.AreEqual("", marks[1], "the wide cell's spacer");
        Assert.AreEqual("\u0300", marks[2]);
    }

    [TestMethod]
    public void AnotherRowLeavesNothingBehind()
    {
        var clusters = new CellClusters();
        Marks("e\u0301", clusters: clusters);
        Assert.IsTrue(Marks("e\r\nx\u0301", clusters: clusters).All(m => m.Length == 0), "row 0 has no cluster now");
    }

    [TestMethod]
    public void OnlyUnicodeMarksAreDrawnOverTheBase()
    {
        Assert.IsTrue(CellClusters.IsMark(0x301), "non-spacing");
        Assert.IsTrue(CellClusters.IsMark(0x20DD), "enclosing");
        Assert.IsTrue(CellClusters.IsMark(0x93E), "spacing combining");
        Assert.IsFalse(CellClusters.IsMark(0x200D), "a joiner waits for shaping");
        Assert.IsFalse(CellClusters.IsMark('a'));
    }

    [TestMethod]
    public void AMarkLeftOfItsPenFollowsTheBase()
    {
        Assert.AreEqual(18, CellClusters.Pen(10, 8, inkLeft: -6, inkWidth: 4), "a zero-width mark drawn back from its pen");
        Assert.AreEqual(10, CellClusters.Pen(10, 8, inkLeft: 1, inkWidth: 4), "a mark drawn inside its own advance");
        Assert.AreEqual(10, CellClusters.Pen(10, 8, inkLeft: -2, inkWidth: 4), "ink centred on the pen stays");
    }
}
