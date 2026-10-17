namespace Scull.Core.Tests;

[TestClass]
public sealed class ConfigTests
{
    private static readonly TimeSpan Patience = TimeSpan.FromSeconds(20);

    /// <summary>A scratch directory per test, so parallel tests never share a watched folder.</summary>
    private sealed class Scratch : IDisposable
    {
        private readonly string dir = Directory.CreateTempSubdirectory("scull-config-").FullName;

        public string File => Path.Combine(dir, "config.toml");

        public void Write(string text) => System.IO.File.WriteAllText(File, text);

        // A reload between an in-place write's truncate and its write would read
        // an empty file and drop the last good values, so edits replace the file.
        public void Replace(string text)
        {
            string draft = Path.Combine(dir, "draft");
            System.IO.File.WriteAllText(draft, text);
            System.IO.File.Move(draft, File, overwrite: true);
        }

        public void Dispose() => Directory.Delete(dir, recursive: true);
    }

    private static ConfigSettings PollUntil(Config config, string what, Func<ConfigSettings, bool> done)
    {
        DateTime end = DateTime.UtcNow + Patience;
        while (true)
        {
            config.TryPoll(out ConfigSettings settings);
            if (done(settings))
            {
                return settings;
            }
            Assert.IsTrue(DateTime.UtcNow < end, $"timed out: {what}");
            Thread.Sleep(20);
        }
    }

    [TestMethod]
    public void AMissingFileIsTheDefaultsAndTheFilesPath()
    {
        using var scratch = new Scratch();
        using var config = Config.Open(scratch.File, () => { });

        Assert.IsNull(config.Settings);
        Assert.IsTrue(config.TryPoll(out ConfigSettings settings));
        Assert.AreSame(settings, config.Settings);
        Assert.AreEqual(scratch.File, settings.Path);
        Assert.AreEqual("", settings.Error);
        Assert.AreEqual("", settings.FontFamily);
        Assert.AreEqual("scull-dark", settings.Scheme);
        Assert.IsTrue(settings.Watching);
        Assert.IsTrue(settings.FontSize > 0);
        Assert.AreEqual(16, settings.Ansi.Count);
        Assert.IsTrue(settings.Keybinds.Count > 0);
        Assert.IsTrue(
            settings.Keybinds.Any(b => b.Action == ConfigAction.Paste && b.Key == 'v' && b.Mods == KeyModifiers.Super),
            "paste is on the platform key");
        Assert.IsTrue(
            settings.Keybinds.Any(b => b.Action == ConfigAction.ScrollPageUp && b.Key == KeyCode.PageUp),
            "bindings are listed by key code");

        Assert.IsFalse(config.TryPoll(out ConfigSettings again), "nothing changed since the last poll");
        Assert.AreSame(settings, again);
    }

    [TestMethod]
    public void AFileOnDiskIsReadAtOpen()
    {
        using var scratch = new Scratch();
        scratch.Write("scrollback = 321\n[font]\nfamily = \"Cascadia Mono\"\nsize = 15.5\n");
        using var config = Config.Open(scratch.File, () => { });

        config.TryPoll(out ConfigSettings settings);

        Assert.AreEqual(321u, settings.Scrollback);
        Assert.AreEqual("Cascadia Mono", settings.FontFamily);
        Assert.AreEqual(15.5f, settings.FontSize);
    }

    [TestMethod]
    public void SetValidatesWritesAndKeepsTheRestOfTheFile()
    {
        using var scratch = new Scratch();
        scratch.Write("# my notes\nscrollback = 99\n");
        using var config = Config.Open(scratch.File, () => { });
        config.TryPoll(out _);

        config.Set("font.size", "17");
        config.Set("colors.scheme", "scull-light");
        Assert.IsTrue(config.TryPoll(out ConfigSettings settings));

        Assert.AreEqual(17f, settings.FontSize);
        Assert.AreEqual("scull-light", settings.Scheme);
        Assert.AreEqual(99u, settings.Scrollback);
        Assert.AreEqual("", settings.Error);
        string file = File.ReadAllText(scratch.File);
        StringAssert.Contains(file, "# my notes");
        StringAssert.Contains(file, "size");

        config.Set("font.size", "");
        config.TryPoll(out settings);
        Assert.AreNotEqual(17f, settings.FontSize, "an empty value brings the default back");
        Assert.IsFalse(File.ReadAllText(scratch.File).Contains("size", StringComparison.Ordinal));
    }

    [TestMethod]
    [DataRow("font.size", "500")]
    [DataRow("font.size", "big")]
    [DataRow("colors.scheme", "no-such-scheme")]
    [DataRow("bogus", "1")]
    public void SetRefusesWhatTheConfigDoesNotAcceptAndLeavesTheFile(string key, string value)
    {
        using var scratch = new Scratch();
        scratch.Write("scrollback = 7\n");
        using var config = Config.Open(scratch.File, () => { });
        config.TryPoll(out _);

        ScullException error = Assert.ThrowsExactly<ScullException>(() => config.Set(key, value));

        Assert.AreEqual(Status.Invalid, error.Status);
        Assert.AreEqual("scrollback = 7\n", File.ReadAllText(scratch.File));
        Assert.IsFalse(config.TryPoll(out _));
    }

    [TestMethod]
    public void AnEditedFileBumpsTheGenerationAndWakesTheHost()
    {
        using var scratch = new Scratch();
        using var woken = new SemaphoreSlim(0);
        using var config = Config.Open(scratch.File, () => woken.Release());
        config.TryPoll(out ConfigSettings first);

        scratch.Replace("[font]\nfamily = \"Consolas\"\nsize = 20\n");

        Assert.IsTrue(woken.Wait(Patience), "the core woke the host");
        ConfigSettings next = PollUntil(config, "the new font", s => s.FontSize == 20f);
        Assert.AreEqual("Consolas", next.FontFamily);
        Assert.IsTrue(next.Generation > first.Generation);

        scratch.Replace("scrollback = 12\n");
        ConfigSettings last = PollUntil(config, "the new scrollback", s => s.Scrollback == 12);
        Assert.IsTrue(last.Generation > next.Generation);
        Assert.AreEqual("", last.FontFamily, "a key the file dropped is back to its default");
    }

    [TestMethod]
    public void ABrokenEditKeepsTheLastGoodValuesAndNamesTheFile()
    {
        using var scratch = new Scratch();
        scratch.Write("scrollback = 3\n");
        using var config = Config.Open(scratch.File, () => { });
        config.TryPoll(out ConfigSettings good);
        Assert.AreEqual(3u, good.Scrollback);

        scratch.Replace("scrollback = -3\n");

        ConfigSettings broken = PollUntil(config, "the error", s => s.Error.Length > 0);
        Assert.AreEqual(3u, broken.Scrollback);
        StringAssert.Contains(broken.Error, "config.toml");
        Assert.IsTrue(broken.Generation > good.Generation, "a new error is a change");

        scratch.Replace("scrollback = 5\n");
        ConfigSettings fixedUp = PollUntil(config, "the repair", s => s.Error.Length == 0 && s.Scrollback == 5);
        Assert.AreEqual(5u, fixedUp.Scrollback);
    }

    [TestMethod]
    public void NoWakeupArrivesAfterDispose()
    {
        using var scratch = new Scratch();
        int wakeups = 0;
        var config = Config.Open(scratch.File, () => Interlocked.Increment(ref wakeups));
        config.TryPoll(out _);

        config.Dispose();
        config.Dispose();
        scratch.Write("scrollback = 1\n");
        Thread.Sleep(400);

        Assert.AreEqual(0, Volatile.Read(ref wakeups));
    }

    [TestMethod]
    public void ACallAfterDisposeThrows()
    {
        using var scratch = new Scratch();
        var config = Config.Open(scratch.File, () => { });
        config.Dispose();

        Assert.ThrowsExactly<ObjectDisposedException>(() => config.TryPoll(out _));
        Assert.ThrowsExactly<ObjectDisposedException>(() => config.Set("scrollback", "1"));
    }

    [TestMethod]
    public void ArgumentsAreChecked()
    {
        using var scratch = new Scratch();
        using var config = Config.Open(scratch.File, () => { });

        Assert.ThrowsExactly<ArgumentNullException>(() => Config.Open(scratch.File, null!));
        Assert.ThrowsExactly<ArgumentNullException>(() => config.Set(null!, "1"));
        Assert.ThrowsExactly<ArgumentNullException>(() => config.Set("scrollback", null!));
    }
}
