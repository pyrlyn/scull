using Scull.Core;
using Scull.Native;

namespace Scull.Render;

/// <summary>
/// Colours as the core hands them out (default, palette index or RGB)
/// resolved to 0xRRGGBB. xterm's palette, as Palette.swift uses, so indexed
/// colours look as they do on macOS until themes arrive (T20.7).
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

    public uint Foreground { get; init; } = 0xE5E5E5;

    public uint Background { get; init; } = 0x141414;

    public uint Cursor { get; init; } = 0xE5E5E5;

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
