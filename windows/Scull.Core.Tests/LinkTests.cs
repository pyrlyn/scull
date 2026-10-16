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

    [TestMethod]
    [DataRow("https://example.com/a?b=1")]
    [DataRow("HTTP://example.com")]
    [DataRow("mailto:someone@example.com")]
    public void WebAndMailLinksOpen(string uri) => Assert.IsNotNull(EventPolicy.OpenableLink(uri, "MYPC"));

    [TestMethod]
    [DataRow("javascript:alert(1)")]
    [DataRow("ftp://example.com")]
    [DataRow("data:text/html,x")]
    [DataRow("ms-settings:")]
    [DataRow("search-ms:query=x")]
    [DataRow("\\\\server\\share\\x")]
    [DataRow("https://")]
    [DataRow("mailto:")]
    [DataRow("file://elsewhere/x/y")]
    [DataRow("not a url")]
    [DataRow("")]
    public void EverythingElseIsIgnored(string uri) => Assert.IsNull(EventPolicy.OpenableLink(uri, "MYPC"));

    [TestMethod]
    public void AFileUrlOpensWhenItNamesThisMachine()
    {
        string local = OperatingSystem.IsWindows() ? "file:///C:/Users/me/a%20b" : "file:///home/me/a%20b";
        string path = OperatingSystem.IsWindows() ? @"C:\Users\me\a b" : "/home/me/a b";
        string named = local.Replace("file:///", "file://mypc/", StringComparison.Ordinal);
        string localhost = local.Replace("file:///", "file://localhost/", StringComparison.Ordinal);
        foreach (string uri in new[] { local, named, localhost })
        {
            Uri? opened = EventPolicy.OpenableLink(uri, "MYPC");
            Assert.IsNotNull(opened, uri);
            Assert.IsTrue(opened.IsFile, uri);
            Assert.AreEqual(path, opened.LocalPath, uri);
        }
    }
}
