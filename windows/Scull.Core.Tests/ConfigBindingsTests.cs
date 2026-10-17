namespace Scull.Core.Tests;

/// <summary>Bindings from the real core matched against the keys Windows reports, and the font a view zooms.</summary>
[TestClass]
public sealed class ConfigBindingsTests
{
    private static readonly uint? V = WindowsKeys.Map(0x56, 0x2F, extended: false);
    private static readonly uint? PageUp = WindowsKeys.Map(0x21, 0x49, extended: true);
    private static readonly uint? Equal = WindowsKeys.Map(0xBB, 0x0D, extended: false);

    private static ConfigSettings Load(string text)
    {
        string dir = Directory.CreateTempSubdirectory("scull-binds-").FullName;
        try
        {
            string file = Path.Combine(dir, "config.toml");
            File.WriteAllText(file, text);
            using var config = Config.Open(file, () => { });
            Assert.IsTrue(config.TryPoll(out ConfigSettings settings));
            Assert.AreEqual("", settings.Error);
            return settings;
        }
        finally
        {
            Directory.Delete(dir, recursive: true);
        }
    }

    [TestMethod]
    public void TheDefaultBindingsMatchTheKeysWindowsReports()
    {
        IReadOnlyList<ConfigKeybind> binds = Load("").Keybinds;

        Assert.AreEqual(ConfigAction.Paste, ConfigBindings.Match(binds, V, KeyModifiers.Super));
        Assert.AreEqual(ConfigAction.ScrollPageUp, ConfigBindings.Match(binds, PageUp, KeyModifiers.Shift));
        Assert.AreEqual(ConfigAction.FontLarger, ConfigBindings.Match(binds, Equal, KeyModifiers.Super | KeyModifiers.Shift));
    }

    [TestMethod]
    public void ModifiersMatchExactlyButLocksAreIgnored()
    {
        IReadOnlyList<ConfigKeybind> binds = Load("").Keybinds;

        Assert.IsNull(ConfigBindings.Match(binds, V, KeyModifiers.Super | KeyModifiers.Control), "an extra modifier");
        Assert.IsNull(ConfigBindings.Match(binds, V, KeyModifiers.None), "plain v is typed");
        Assert.AreEqual(ConfigAction.Paste, ConfigBindings.Match(binds, V, KeyModifiers.Super | KeyModifiers.CapsLock | KeyModifiers.NumLock));
        Assert.IsNull(ConfigBindings.Match(binds, null, KeyModifiers.Super), "a key the core has no name for");
    }

    [TestMethod]
    public void AConfiguredBindingReplacesOrDisablesADefault()
    {
        IReadOnlyList<ConfigKeybind> binds = Load(
            "[[keybind]]\nkey = \"ctrl+=\"\naction = \"font-larger\"\n" +
            "[[keybind]]\nkey = \"super+v\"\naction = \"none\"\n").Keybinds;

        Assert.AreEqual(ConfigAction.FontLarger, ConfigBindings.Match(binds, Equal, KeyModifiers.Control));
        Assert.IsNull(ConfigBindings.Match(binds, V, KeyModifiers.Super), "none unbinds it");
    }

    [TestMethod]
    public void ScrollActionsMoveAPageOrToEitherEnd()
    {
        Assert.AreEqual(24, ConfigBindings.ScrollRows(ConfigAction.ScrollPageUp, 24));
        Assert.AreEqual(-24, ConfigBindings.ScrollRows(ConfigAction.ScrollPageDown, 24));
        Assert.AreEqual(int.MaxValue, ConfigBindings.ScrollRows(ConfigAction.ScrollToTop, 24));
        Assert.AreEqual(-int.MaxValue, ConfigBindings.ScrollRows(ConfigAction.ScrollToBottom, 24));
        Assert.IsNull(ConfigBindings.ScrollRows(ConfigAction.Paste, 24));
    }

    [TestMethod]
    public void ScrollingToTheTopReachesTheOldestRow()
    {
        using var terminal = new Terminal(10, 2);
        terminal.Feed("one\r\ntwo\r\nthree\r\nfour"u8);
        terminal.Update();
        terminal.ScrollDisplay(ConfigBindings.ScrollRows(ConfigAction.ScrollToTop, 2)!.Value);
        terminal.Update();
        Assert.AreEqual('o', (char)terminal.View.Cells[0].CodePoint);
    }

    [TestMethod]
    public void ZoomStepsByOneAndResets()
    {
        var font = new FontZoom("Consolas", 13);
        Assert.IsTrue(font.Run(ConfigAction.FontLarger));
        Assert.IsTrue(font.Run(ConfigAction.FontLarger));
        Assert.AreEqual(15f, font.Size);
        Assert.IsTrue(font.Run(ConfigAction.FontSmaller));
        Assert.AreEqual(14f, font.Size);
        Assert.IsTrue(font.Run(ConfigAction.FontReset));
        Assert.AreEqual(13f, font.Size);
        Assert.IsFalse(font.Run(ConfigAction.Paste));
    }

    [TestMethod]
    public void ZoomStopsAtTheLimitsAndStepsBackAtOnce()
    {
        var font = new FontZoom("", 5);
        Assert.IsTrue(font.Run(ConfigAction.FontSmaller));
        Assert.IsFalse(font.Run(ConfigAction.FontSmaller), "4 is the floor");
        Assert.AreEqual(FontZoom.MinSize, font.Size);
        Assert.IsTrue(font.Run(ConfigAction.FontLarger));
        Assert.AreEqual(5f, font.Size, "one step up from the floor");

        var big = new FontZoom("", 199);
        big.Run(ConfigAction.FontLarger);
        big.Run(ConfigAction.FontLarger);
        Assert.AreEqual(FontZoom.MaxSize, big.Size);
    }

    [TestMethod]
    public void ANewConfiguredFontDropsTheZoom()
    {
        var font = new FontZoom();
        Assert.IsTrue(font.Apply(new ConfigSettings { FontFamily = "Consolas", FontSize = 13 }));
        font.Run(ConfigAction.FontLarger);

        Assert.IsFalse(font.Apply(new ConfigSettings { FontFamily = "Consolas", FontSize = 13, Scheme = "light" }), "only the colours changed");
        Assert.AreEqual(14f, font.Size, "the zoom survives");
        Assert.IsTrue(font.Apply(new ConfigSettings { FontFamily = "Consolas", FontSize = 16 }));
        Assert.AreEqual(16f, font.Size);
    }

    [TestMethod]
    public void TheConfiguredFamilyComesBeforeTheSystemsMonospaceFonts()
    {
        Assert.AreEqual("Cascadia Mono,Consolas", string.Join(",", new FontZoom().Families), "empty is the system font");
        Assert.AreEqual("Fira Code,Cascadia Mono,Consolas", string.Join(",", new FontZoom("Fira Code").Families));
    }
}
