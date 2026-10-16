using System.Globalization;

namespace Scull.Core;

/// <summary>One setting to write: a config key and its value; an empty value removes the key.</summary>
public readonly record struct SettingChange(string Key, string Value);

/// <summary>
/// What the settings window shows and writes, apart from the toolkit: the keys, the choices, the
/// bounds the core enforces and the checks that turn a typed value into a <see cref="SettingChange"/>.
/// The core still has the last word (<see cref="Write"/> reports its refusal); these checks only keep
/// the window from sending what it knows would be refused, with a message the user can act on.
/// </summary>
public static class SettingsForm
{
    public const string FontFamilyKey = "font.family";
    public const string FontSizeKey = "font.size";
    public const string SchemeKey = "colors.scheme";
    public const string ScrollbackKey = "scrollback";

    // scull_config::{MIN,MAX}_FONT_SIZE and MAX_SCROLLBACK; the core refuses anything outside.
    public const double MinFontSize = 4;
    public const double MaxFontSize = 200;
    public const double FontSizeStep = 0.5;
    public const double MaxScrollback = 1_000_000;

    // The built-in schemes, as macos/Sources/Scull/SettingsView.swift lists them.
    private static readonly string[] BuiltInSchemes = ["scull-dark", "scull-light", "solarized-dark"];

    /// <summary>The scheme names to offer, then <paramref name="current"/> if the file names another one.</summary>
    public static IReadOnlyList<string> Schemes(string current) =>
        current.Length == 0 || BuiltInSchemes.Contains(current, StringComparer.Ordinal) ? BuiltInSchemes : [.. BuiltInSchemes, current];

    /// <summary>A font family; empty is the system's monospace font.</summary>
    public static SettingChange Family(string family) => new(FontFamilyKey, family.Trim());

    public static SettingChange Scheme(string scheme) => new(SchemeKey, scheme);

    /// <summary>The size in points, written without a trailing zero (<c>14</c>, <c>13.5</c>).</summary>
    public static bool TryFontSize(double size, out SettingChange change, out string problem)
    {
        change = default;
        if (!double.IsFinite(size) || size < MinFontSize || size > MaxFontSize)
        {
            problem = string.Create(CultureInfo.InvariantCulture, $"A font size is {MinFontSize} to {MaxFontSize} points.");
            return false;
        }
        change = new SettingChange(FontSizeKey, size.ToString("0.##", CultureInfo.InvariantCulture));
        problem = "";
        return true;
    }

    /// <summary>History rows, a whole number from 0.</summary>
    public static bool TryScrollback(double rows, out SettingChange change, out string problem)
    {
        change = default;
        if (!double.IsFinite(rows) || rows < 0 || rows > MaxScrollback || rows != Math.Floor(rows))
        {
            problem = string.Create(CultureInfo.InvariantCulture, $"Scrollback is a whole number of rows from 0 to {MaxScrollback:N0}.");
            return false;
        }
        change = new SettingChange(ScrollbackKey, ((uint)rows).ToString(CultureInfo.InvariantCulture));
        problem = "";
        return true;
    }

    /// <summary>
    /// Writes a change through <see cref="Config.Set"/>. Null when it went in; otherwise the
    /// core's message for a value it refused or a file it could not write, with the file untouched.
    /// </summary>
    public static string? Write(Config config, SettingChange change)
    {
        ArgumentNullException.ThrowIfNull(config);
        try
        {
            config.Set(change.Key, change.Value);
            return null;
        }
        catch (ScullException e) when (e.Status is Status.Invalid or Status.Io)
        {
            return e.Message;
        }
    }
}
