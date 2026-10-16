using System.Text;

namespace Scull.Core.Tests;

[TestClass]
public sealed class EventTests
{
    private static List<TerminalEvent> Events(string output)
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed(Encoding.UTF8.GetBytes(output));
        var found = new List<TerminalEvent>();
        while (terminal.TryPollEvent(out TerminalEvent next))
        {
            found.Add(next);
        }
        return found;
    }

    [TestMethod]
    public void ATitleComesWithItsText()
    {
        TerminalEvent both = Events("\e]0;vim — notes\a").Single();
        Assert.AreEqual(EventKind.Title, both.Kind);
        Assert.AreEqual("vim — notes", both.Text);
        Assert.AreEqual("vim — notes", EventPolicy.TitleOf(both));

        TerminalEvent window = Events("\e]2;build\e\\").Single();
        Assert.AreEqual("build", EventPolicy.TitleOf(window));
    }

    [TestMethod]
    public void AnIconNameIsNotAWindowTitle()
    {
        TerminalEvent icon = Events("\e]1;icon\a").Single();
        Assert.AreEqual(EventKind.Title, icon.Kind);
        Assert.IsNull(EventPolicy.TitleOf(icon));
    }

    [TestMethod]
    public void ATitleLosesItsControlCharacters()
    {
        TerminalEvent title = new(EventKind.Title, null, false, Body: "a\nb\u009bc\td"u8.ToArray());
        Assert.AreEqual("abcd", EventPolicy.TitleOf(title));
    }

    [TestMethod]
    public void OnlyATitleEventIsATitle()
    {
        Assert.IsNull(EventPolicy.TitleOf(new TerminalEvent(EventKind.Bell, null, false)));
    }

    [TestMethod]
    public void TheWorkingDirectoryIsTheUrlTheProgramSent()
    {
        TerminalEvent cwd = Events("\e]7;file:///home/me/my%20dir\a").Single();
        Assert.AreEqual(EventKind.WorkingDirectory, cwd.Kind);
        Assert.AreEqual("file:///home/me/my%20dir", cwd.Text);
    }

    [TestMethod]
    public void AClipboardWriteCarriesItsDecodedBytes()
    {
        string base64 = Convert.ToBase64String("héllo"u8);
        TerminalEvent write = Events($"\e]52;c;{base64}\a").Single();
        Assert.AreEqual(EventKind.ClipboardWrite, write.Kind);
        Assert.AreEqual((uint)'c', write.Detail);
        CollectionAssert.AreEqual("héllo"u8.ToArray(), write.Body);
        Assert.AreEqual("héllo", EventPolicy.ClipboardText(write));
    }

    [TestMethod]
    public void AClipboardWriteThatIsNotUtf8IsDropped()
    {
        string base64 = Convert.ToBase64String([0xFF, 0xFE, 0x41]);
        TerminalEvent write = Events($"\e]52;c;{base64}\a").Single();
        CollectionAssert.AreEqual(new byte[] { 0xFF, 0xFE, 0x41 }, write.Body);
        Assert.IsNull(EventPolicy.ClipboardText(write));
    }

    [TestMethod]
    public void AClipboardReadIsARequestThatCanBeDenied()
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed("\e]52;c;?\a"u8);
        Assert.IsTrue(terminal.TryPollEvent(out TerminalEvent read));
        Assert.AreEqual(EventKind.ClipboardRead, read.Kind);
        Assert.AreNotEqual(0UL, read.Id);
        Assert.IsNull(EventPolicy.ClipboardText(read), "a read has nothing to write");

        Assert.IsTrue(terminal.ClipboardDeny(read.Id));
    }

    [TestMethod]
    public void LongTextIsReadInFull()
    {
        byte[] bytes = Enumerable.Range(0, 100_000).Select(i => (byte)('a' + i % 26)).ToArray();
        TerminalEvent write = Events($"\e]52;c;{Convert.ToBase64String(bytes)}\a").Single();
        CollectionAssert.AreEqual(bytes, write.Body);
    }

    [TestMethod]
    public void EventsComeInTheOrderTheyWereSent()
    {
        var kinds = Events("\e]2;a\a\e]7;file:///x\a\a").Select(e => e.Kind).ToArray();
        CollectionAssert.AreEqual(new[] { EventKind.Title, EventKind.WorkingDirectory, EventKind.Bell }, kinds);
    }

    [TestMethod]
    [DataRow("file:///home/me/proj", "/home/me/proj")]
    [DataRow("file://localhost/home/me", "/home/me")]
    [DataRow("file:///home/me/a%20b", "/home/me/a b")]
    [DataRow("file://elsewhere/home/me", null)]
    [DataRow("file:////h/s/x", null)]
    [DataRow("file://localhost//h/s/x", null)]
    [DataRow("https://example.com/x", null)]
    [DataRow("/home/me", null)]
    [DataRow("", null)]
    public void OnlyALocalFileUrlIsAPath(string url, string? path)
    {
        if (!OperatingSystem.IsWindows())
        {
            Assert.AreEqual(path, EventPolicy.PathOfUrl(url));
        }
    }

    [TestMethod]
    public void AWindowsFileUrlIsADrivePath()
    {
        if (OperatingSystem.IsWindows())
        {
            Assert.AreEqual(@"C:\Users\me\a b", EventPolicy.PathOfUrl("file:///C:/Users/me/a%20b"));
            Assert.IsNull(EventPolicy.PathOfUrl("file://server/share/x"));
            Assert.IsNull(EventPolicy.PathOfUrl("file:////server/share/x"), "an empty host and a share path");
            Assert.IsNull(EventPolicy.PathOfUrl("file://localhost//server/share/x"));
            Assert.IsNull(EventPolicy.PathOfUrl("file:///%5C%5Cserver/share"));
            Assert.IsNull(EventPolicy.PathOfUrl("file:///\\\\server\\share"));
        }
    }
}

[TestClass]
public sealed class PaneEventsTests
{
    private static DrainedEvents Drain(PaneEvents events, string output)
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed(Encoding.UTF8.GetBytes(output));
        return events.Drain(terminal);
    }

    [TestMethod]
    public void TheTitleAndDirectoryAreKept()
    {
        var events = new PaneEvents();
        DrainedEvents drained = Drain(events, "\e]2;build\a\e]7;file:///work/app\a");
        Assert.IsTrue(drained.TitleChanged);
        Assert.AreEqual("build", events.Title);
        if (!OperatingSystem.IsWindows())
        {
            Assert.AreEqual("/work/app", events.WorkingDirectory);
        }
        Assert.IsFalse(drained.Exited);
        Assert.IsNull(drained.Clipboard);
    }

    [TestMethod]
    public void ARepeatedTitleIsNoChange()
    {
        var events = new PaneEvents();
        Drain(events, "\e]2;same\a");
        Assert.IsFalse(Drain(events, "\e]2;same\a").TitleChanged);
        Assert.IsTrue(Drain(events, "\e]2;other\a").TitleChanged);
    }

    [TestMethod]
    public void AnEmptyTitleClearsIt()
    {
        var events = new PaneEvents();
        Drain(events, "\e]2;named\a");
        Assert.IsTrue(Drain(events, "\e]2;\a").TitleChanged);
        Assert.IsNull(events.Title);
    }

    [TestMethod]
    public void AnIconNameAndAForeignDirectoryChangeNothing()
    {
        var events = new PaneEvents();
        DrainedEvents drained = Drain(events, "\e]1;icon\a\e]7;file://elsewhere/etc\a");
        Assert.IsFalse(drained.TitleChanged);
        Assert.IsNull(events.Title);
        Assert.IsNull(events.WorkingDirectory);
    }

    [TestMethod]
    public void TheLastClipboardWriteWinsAndOnlyUtf8Counts()
    {
        var events = new PaneEvents();
        string Osc(byte[] bytes) => $"\e]52;c;{Convert.ToBase64String(bytes)}\a";
        Assert.AreEqual("two", Drain(events, Osc("one"u8.ToArray()) + Osc("two"u8.ToArray())).Clipboard);
        Assert.AreEqual("kept", Drain(events, Osc("kept"u8.ToArray()) + Osc([0xFF])).Clipboard, "bad bytes do not replace good text");
        Assert.IsNull(Drain(events, Osc([0xFF])).Clipboard);
    }

    [TestMethod]
    public void AClipboardReadIsDeniedAndGivesNothing()
    {
        var events = new PaneEvents();
        DrainedEvents drained = Drain(events, "\e]52;c;?\a");
        Assert.IsNull(drained.Clipboard);
        Assert.IsFalse(drained.TitleChanged);
    }

    [TestMethod]
    public void AClipboardReadIsAnsweredWithADenialTheChildCanSee()
    {
        if (OperatingSystem.IsWindows())
        {
            return;
        }
        using Terminal terminal = Terminal.Spawn(80, 24, () => { },
            new SpawnOptions { Program = "/bin/sh", Arguments = ["-c", "stty raw -echo; printf '\\033]52;c;?\\007'; dd bs=1 count=1 2>/dev/null; echo done"] });
        var events = new PaneEvents();
        bool exited = false;
        for (int i = 0; i < 200 && !exited; i++)
        {
            exited = events.Drain(terminal).Exited;
            Thread.Sleep(25);
        }
        Assert.IsTrue(exited, "the child finished: the denial reached it");
    }
}
