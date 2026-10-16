namespace Scull.Core.Tests;

/// <summary>OSC 8 links through <see cref="Terminal"/>, as LinkTests.swift checks them.</summary>
[TestClass]
public sealed class LinkTests
{
    [TestMethod]
    public void ALinkedCellAnswersItsUriAndHoveringFlagsItsCells()
    {
        using var terminal = new Terminal(8, 2);
        // Longer than the first buffer, so the copy has to grow it.
        string uri = "https://example.com/" + new string('a', 300);
        terminal.Feed(System.Text.Encoding.UTF8.GetBytes($"x\e]8;;{uri}\e\\ab\e]8;;\e\\"));
        Assert.IsNull(terminal.LinkAt(0, 0));
        uint id = terminal.LinkAt(0, 1) ?? throw new AssertFailedException("no link at column 1");
        Assert.AreEqual(id, terminal.LinkAt(0, 2));
        Assert.AreEqual(uri, terminal.LinkUri(id));
        Assert.IsNull(terminal.LinkUri(id + 1));
        Assert.IsNull(terminal.LinkAt(5, 1), "below the viewport");

        terminal.HoverLink(id);
        terminal.Update();
        Assert.AreEqual("-LL-", string.Concat(terminal.View.Cells[..4].ToArray().Select(c => c.IsLink ? 'L' : '-')));
        terminal.HoverLink(null);
        terminal.Update();
        Assert.IsFalse(terminal.View.Cells[1].IsLink);
    }
}
