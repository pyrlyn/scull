namespace Scull.Core.Tests;

[TestClass]
public sealed class SettingsFormTests
{
    private static readonly string[] BuiltIn = ["scull-dark", "scull-light", "solarized-dark"];

    private sealed class Scratch : IDisposable
    {
        private readonly string dir = Directory.CreateTempSubdirectory("scull-settings-").FullName;

        public string File => Path.Combine(dir, "config.toml");

        public void Dispose() => Directory.Delete(dir, recursive: true);
    }

    [TestMethod]
    [DataRow(14.0, "14")]
    [DataRow(13.5, "13.5")]
    [DataRow(4.0, "4")]
    [DataRow(200.0, "200")]
    public void ASizeInRangeIsWrittenWithoutATrailingZero(double size, string written)
    {
        Assert.IsTrue(SettingsForm.TryFontSize(size, out SettingChange change, out string problem), problem);
        Assert.AreEqual(new SettingChange("font.size", written), change);
    }

    [TestMethod]
    [DataRow(3.9)]
    [DataRow(200.5)]
    [DataRow(double.NaN)]
    [DataRow(double.PositiveInfinity)]
    [DataRow(-1.0)]
    public void ASizeOutOfRangeIsRefusedWithAMessage(double size)
    {
        Assert.IsFalse(SettingsForm.TryFontSize(size, out SettingChange change, out string problem));
        Assert.AreEqual(default, change);
        StringAssert.Contains(problem, "4 to 200");
    }

    [TestMethod]
    [DataRow(0.0, "0")]
    [DataRow(10_000.0, "10000")]
    [DataRow(1_000_000.0, "1000000")]
    public void ScrollbackIsAWholeNumberOfRows(double rows, string written)
    {
        Assert.IsTrue(SettingsForm.TryScrollback(rows, out SettingChange change, out string problem), problem);
        Assert.AreEqual(new SettingChange("scrollback", written), change);
    }

    [TestMethod]
    [DataRow(-1.0)]
    [DataRow(1_000_001.0)]
    [DataRow(2.5)]
    [DataRow(double.NaN)]
    public void ScrollbackOutOfRangeOrFractionalIsRefused(double rows)
    {
        Assert.IsFalse(SettingsForm.TryScrollback(rows, out _, out string problem));
        StringAssert.Contains(problem, "whole number");
    }

    [TestMethod]
    public void TheSchemeListKeepsAFilesOwnSchemeSelectable()
    {
        CollectionAssert.AreEqual(BuiltIn, SettingsForm.Schemes("scull-light").ToArray());
        Assert.AreEqual("mine", SettingsForm.Schemes("mine")[^1]);
        Assert.HasCount(3, SettingsForm.Schemes(""));
    }

    [TestMethod]
    public void ABuiltInSchemeIsOneTheCoreAccepts()
    {
        using var scratch = new Scratch();
        using var config = Config.Open(scratch.File, () => { });
        foreach (string scheme in SettingsForm.Schemes(""))
        {
            Assert.IsNull(SettingsForm.Write(config, SettingsForm.Scheme(scheme)), scheme);
            config.TryPoll(out ConfigSettings settings);
            Assert.AreEqual(scheme, settings.Scheme);
        }
    }

    [TestMethod]
    public void WritingGoesThroughTheCoreAndItsRefusalComesBackAsAMessage()
    {
        using var scratch = new Scratch();
        using var config = Config.Open(scratch.File, () => { });

        Assert.IsTrue(SettingsForm.TryFontSize(18.5, out SettingChange size, out _));
        Assert.IsNull(SettingsForm.Write(config, size));
        Assert.IsNull(SettingsForm.Write(config, SettingsForm.Family("  Consolas ")));
        config.TryPoll(out ConfigSettings settings);
        Assert.AreEqual(18.5f, settings.FontSize);
        Assert.AreEqual("Consolas", settings.FontFamily);

        string before = File.ReadAllText(scratch.File);
        string? refusal = SettingsForm.Write(config, SettingsForm.Scheme("no-such-scheme"));
        Assert.IsFalse(string.IsNullOrEmpty(refusal));
        Assert.AreEqual(before, File.ReadAllText(scratch.File), "a refused value leaves the file alone");
    }

    [TestMethod]
    public void AnEmptyFamilyRemovesTheKeySoTheSystemMonospaceApplies()
    {
        using var scratch = new Scratch();
        using var config = Config.Open(scratch.File, () => { });
        Assert.IsNull(SettingsForm.Write(config, SettingsForm.Family("Consolas")));
        Assert.IsNull(SettingsForm.Write(config, SettingsForm.Family("")));
        config.TryPoll(out ConfigSettings settings);
        Assert.AreEqual("", settings.FontFamily);
    }
}
