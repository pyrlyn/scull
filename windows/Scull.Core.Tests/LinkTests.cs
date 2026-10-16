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
    [DataRow("https://example.com/a?b=1", LinkKind.Web)]
    [DataRow("mailto:someone@example.com", LinkKind.Mail)]
    [DataRow("javascript:alert(1)", LinkKind.Refused)]
    [DataRow("https://bank.example@evil.example", LinkKind.Refused)]
    [DataRow("file:////h/s/x", LinkKind.Refused)]
    [DataRow("file://localhost//h/s/x", LinkKind.Refused)]
    [DataRow("file:///%5C%5Ch/s", LinkKind.Refused)]
    public void TheCoreSortsLinks(string uri, LinkKind kind)
    {
        LinkTarget target = LinkTarget.Of(uri, "MYPC");
        Assert.AreEqual(kind, target.Kind);
        Assert.AreEqual(kind is LinkKind.Refused ? "" : uri, target.Target);
    }

    [TestMethod]
    public void ALocalDirectoryOpensAndAFileIsOnlyRevealed()
    {
        string dir = Directory.CreateTempSubdirectory("scull-link").FullName;
        try
        {
            string file = Path.Combine(dir, "a b.cmd");
            File.WriteAllText(file, "x");
            string Url(string path) => new Uri(path).AbsoluteUri;
            LinkTarget folder = LinkTarget.Of(Url(dir), Environment.MachineName);
            Assert.AreEqual(LinkKind.Directory, folder.Kind);
            Assert.AreEqual(Path.GetFullPath(dir), Path.GetFullPath(folder.Target));
            LinkTarget script = LinkTarget.Of(Url(file), Environment.MachineName);
            Assert.AreEqual(LinkKind.File, script.Kind, "a script is revealed, not run");
            Assert.AreEqual(Path.GetFullPath(file), Path.GetFullPath(script.Target));
            Assert.AreEqual(LinkKind.Refused, LinkTarget.Of(Url(Path.Combine(dir, "missing")), "").Kind);
        }
        finally
        {
            Directory.Delete(dir, recursive: true);
        }
    }
}
