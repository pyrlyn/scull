using Scull.Core;
using Scull.Native;

namespace Scull.Render;

/// <summary>
/// Colours as the core hands them out (default, palette index or RGB)
/// resolved to 0xRRGGBB. xterm's palette, as Palette.swift uses, with the
/// configured colours over it.
/// </summary>
public sealed class Palette
{
    private readonly uint[] indexed = new uint[256];

    public Palette()
    {
        uint[] ansi =
        [
            0x000000, 0xCD0000, 0x00CD00, 0xCDCD00, 0x0000EE, 0xCD00CD, 0x00CDCD, 0xE5E5E5,
            0x7F7F7F, 0xFF0000, 0x00FF00, 0xFFFF00, 0x5C5CFF, 0xFF00FF, 0x00FFFF, 0xFFFFFF,
        ];
        ansi.CopyTo(indexed, 0);
        // xterm's cube levels (256colres.pl), which most terminals copy.
        ReadOnlySpan<uint> levels = [0, 95, 135, 175, 215, 255];
        for (int i = 0; i < 216; i++)
        {
            indexed[16 + i] = levels[i / 36] << 16 | levels[i / 6 % 6] << 8 | levels[i % 6];
        }
        for (uint i = 0; i < 24; i++)
        {
            uint level = 8 + 10 * i;
            indexed[232 + i] = level << 16 | level << 8 | level;
        }
    }

    /// <summary>The configured colours, as Palette+Config.swift lays them over xterm's.</summary>
    /// <param name="accent">The system accent colour, 0xRRGGBB; null keeps the built-in selection colour.</param>
    public Palette(ConfigSettings settings, uint? accent = null)
        : this()
    {
        ArgumentNullException.ThrowIfNull(settings);
        (Foreground, Background, Cursor) = (settings.Foreground, settings.Background, settings.Cursor);
        // The config has no selection colour yet; the accent, toned down over
        // the background, keeps the text on it readable.
        if (accent is uint rgb)
        {
            Selection = Blend(rgb, Background, 0.45);
        }
        // The core sends the sixteen it resolved; the 6x6x6 cube and the greys stay xterm's.
        for (int i = 0; i < Math.Min(16, settings.Ansi.Count); i++)
        {
            indexed[i] = settings.Ansi[i] & 0xFF_FFFF;
        }
    }

    public uint Foreground { get; init; } = 0xE5E5E5;

    public uint Background { get; init; } = 0x141414;

    public uint Cursor { get; init; } = 0xE5E5E5;

    public uint Selection { get; init; } = 0x264F78;

    public uint Match { get; init; } = 0x6B5A1A;

    public uint CurrentMatch { get; init; } = 0xF2A33A;

    /// <summary>Text on the current match, whose background is bright in any theme: black.</summary>
    public uint CurrentMatchText { get; init; }

    /// <summary>The background a cell shows: the current match over the selection over other matches over its own <paramref name="bg"/>.</summary>
    public uint Highlight(in Cell cell, uint bg) =>
        cell.IsCurrentMatch ? CurrentMatch : cell.IsSelected ? Selection : cell.IsMatch ? Match : bg;

    /// <summary><paramref name="top"/> over <paramref name="bottom"/> at <paramref name="alpha"/>, per channel, rounded.</summary>
    public static uint Blend(uint top, uint bottom, double alpha)
    {
        uint rgb = 0;
        for (int shift = 16; shift >= 0; shift -= 8)
        {
            double t = top >> shift & 0xFF, b = bottom >> shift & 0xFF;
            rgb |= (uint)Math.Round(t * alpha + b * (1 - alpha), MidpointRounding.AwayFromZero) << shift;
        }
        return rgb;
    }

    /// <summary>The RGB value of a core colour, <paramref name="fallback"/> for the default.</summary>
    public uint Rgb(uint color, uint fallback) => (color & NativeMethods.TT_COLOR_KIND_MASK) switch
    {
        NativeMethods.TT_COLOR_INDEXED => indexed[color & 0xFF],
        NativeMethods.TT_COLOR_RGB => color & 0xFF_FFFF,
        _ => fallback,
    };

    /// <summary>Foreground and background of a style, reverse video applied.</summary>
    public (uint Fg, uint Bg) Colors(in Style style)
    {
        uint fg = Rgb(style.Foreground, Foreground);
        uint bg = Rgb(style.Background, Background);
        return (style.Attributes & StyleAttributes.Inverse) != 0 ? (bg, fg) : (fg, bg);
    }
}
