using System.Runtime.InteropServices;
using Scull.Native;

namespace Scull.Core;

/// <summary>
/// The frame as the last <see cref="Terminal.Update"/> left it. The spans
/// point into the frame's own buffers: they are valid until the next update or
/// until the terminal is disposed, which is why this is a ref struct.
/// </summary>
public readonly unsafe ref struct FrameView
{
    private readonly tt_frame_view raw;

    internal FrameView(in tt_frame_view raw) => this.raw = raw;

    public ushort Cols => raw.cols;

    public ushort Rows => raw.rows;

    /// <summary>False while synchronized output holds the picture: keep showing the last one.</summary>
    public bool Updated => raw.updated != 0;

    /// <summary>Every row must be repainted; each is also in <see cref="Dirty"/>.</summary>
    public bool Full => raw.full != 0;

    public Cursor Cursor => new(raw.cursor.row, raw.cursor.col, raw.cursor.visible != 0);

    /// <summary><c>Cols * Rows</c> cells, row-major.</summary>
    public ReadOnlySpan<Cell> Cells => Span<Cell>(raw.cells, raw.cells_len);

    /// <summary>One entry per viewport row.</summary>
    public ReadOnlySpan<Row> Lines => Span<Row>(raw.lines, raw.lines_len);

    /// <summary>The style table cells and runs index; entry 0 is the default.</summary>
    public ReadOnlySpan<Style> Styles => Span<Style>(raw.styles, raw.styles_len);

    /// <summary>Rows to blit from the previous picture, applied before <see cref="Dirty"/>.</summary>
    public ReadOnlySpan<Scroll> Scrolls => Span<Scroll>(raw.scrolls, raw.scrolls_len);

    /// <summary>Where the input method's composing text sits; <see cref="Preedit.Cols"/> is 0 when none is shown.</summary>
    public Preedit Preedit => new(raw.preedit.row, raw.preedit.col, raw.preedit.cols);

    /// <summary>Rows to repaint, top to bottom.</summary>
    public ReadOnlySpan<ushort> Dirty => Span<ushort>(raw.dirty, raw.dirty_len);

    /// <summary>The images in the viewport, lowest <see cref="Placement.Z"/> first.</summary>
    public ReadOnlySpan<Placement> Placements => Span<Placement>(raw.placements, raw.placements_len);

    /// <summary>The style at <paramref name="index"/>, or the default one when the index is out of the table.</summary>
    public Style StyleAt(ushort index) => index < Styles.Length ? Styles[index] : default;

    // The core's lengths still describe data the child wrote, so a length
    // past int is refused rather than truncated.
    internal static ReadOnlySpan<T> Span<T>(void* pointer, nuint length)
        where T : unmanaged => new(pointer, checked((int)length));
}

public readonly record struct Cursor(ushort Row, ushort Col, bool Visible);

/// <summary>The span of composing text in the frame: already in the row's cells, so a renderer only marks it.</summary>
public readonly record struct Preedit(ushort Row, ushort Col, ushort Cols);

/// <summary>One grid cell. Same layout as <c>tt_cell</c>, so a span of them is the core's buffer.</summary>
[StructLayout(LayoutKind.Sequential)]
public readonly struct Cell
{
    private readonly tt_cell raw;

    /// <summary>The first code point of the cell's text; 0 when it shows none.</summary>
    public uint CodePoint => raw.codepoint;

    public ushort Style => raw.style;

    /// <summary>Columns covered: 1, 2 for a wide character, 0 for the cell behind it.</summary>
    public byte Width => raw.width;

    /// <summary>The cell holds more code points than <see cref="CodePoint"/>; the cluster is in its row's text.</summary>
    public bool IsCluster => (raw.flags & NativeMethods.TT_CELL_CLUSTER) != 0;

    /// <summary>The cell is inside the selection (and in the viewport: the frame marks nothing scrolled out of it).</summary>
    public bool IsSelected => (raw.flags & NativeMethods.TT_CELL_SELECTED) != 0;

    /// <summary>The cell is part of a search match; also set for the current one.</summary>
    public bool IsMatch => (raw.flags & NativeMethods.TT_CELL_MATCH) != 0;

    /// <summary>The cell is part of the match the last search step went to.</summary>
    public bool IsCurrentMatch => (raw.flags & NativeMethods.TT_CELL_CURRENT_MATCH) != 0;
}

/// <summary>One viewport row: a stable id, a generation that changes with its content, and its text runs.</summary>
[StructLayout(LayoutKind.Sequential)]
public readonly unsafe struct Row
{
    private readonly tt_row raw;

    public ulong Id => raw.id;

    public uint Generation => raw.generation;

    public ReadOnlySpan<Run> Runs => FrameView.Span<Run>(raw.runs, raw.runs_len);

    /// <summary>The row's UTF-8 text that runs slice.</summary>
    public ReadOnlySpan<byte> Text => FrameView.Span<byte>(raw.text, raw.text_len);

    /// <summary>The UTF-8 text of <paramref name="run"/>; empty when its slice is out of the row's text.</summary>
    public ReadOnlySpan<byte> TextOf(Run run)
    {
        ReadOnlySpan<byte> text = Text;
        ulong end = (ulong)run.TextStart + run.TextLength;
        return end <= (ulong)text.Length ? text.Slice((int)run.TextStart, (int)run.TextLength) : default;
    }
}

/// <summary>Neighbouring cells of one style and width with their text: the unit the shaper takes.</summary>
[StructLayout(LayoutKind.Sequential)]
public readonly struct Run
{
    private readonly tt_run raw;

    public ushort Col => raw.col;

    public ushort Cols => raw.cols;

    public ushort Style => raw.style;

    /// <summary>Columns per character: 1 or 2.</summary>
    public byte Width => raw.width;

    public uint TextStart => raw.text_start;

    public uint TextLength => raw.text_len;
}

/// <summary>
/// A style as SGR set it, palette and reverse video not resolved. Colours are packed as in scull.h: <c>TT_COLOR_DEFAULT</c>,
/// <c>TT_COLOR_INDEXED | index</c> or <c>TT_COLOR_RGB | 0xRRGGBB</c>.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public readonly struct Style
{
    private readonly tt_style raw;

    public uint Foreground => raw.fg;

    public uint Background => raw.bg;

    public uint UnderlineColor => raw.underline_color;

    public StyleAttributes Attributes => (StyleAttributes)raw.attrs;

    public Underline Underline => (Underline)raw.underline;
}

/// <summary>
/// One image in the viewport: the source rectangle of its pixels scaled to
/// <see cref="Cols"/> x <see cref="Rows"/> cells from (<see cref="Row"/>,
/// <see cref="Col"/>), moved by the pixel offset. Same layout as <c>tt_placement</c>.
/// </summary>
[StructLayout(LayoutKind.Sequential)]
public readonly unsafe struct Placement
{
    private readonly tt_placement raw;

    /// <summary>The image's identity while the view is valid; with <see cref="Generation"/> it keys a texture cache.</summary>
    public nint Image => (nint)raw.image;

    /// <summary>Changes whenever the pixels behind <see cref="Image"/> may have.</summary>
    public ulong Generation => raw.generation;

    /// <summary>Viewport row of the top-left cell; negative above the viewport.</summary>
    public int Row => raw.row;

    public uint Col => raw.col;

    public uint Cols => raw.cols;

    public uint Rows => raw.rows;

    public uint OffsetX => raw.offset_x;

    public uint OffsetY => raw.offset_y;

    public uint SourceX => raw.src_x;

    public uint SourceY => raw.src_y;

    public uint SourceWidth => raw.src_w;

    public uint SourceHeight => raw.src_h;

    /// <summary>Stacking order; a negative one draws below the text.</summary>
    public int Z => raw.z;

    /// <summary>
    /// The image's pixels: <paramref name="height"/> rows of <paramref name="stride"/> bytes, each
    /// <paramref name="width"/> RGBA pixels with straight alpha. Valid while the view is; empty, with zero sizes, when
    /// there are none or the sizes do not describe a buffer.
    /// </summary>
    public ReadOnlySpan<byte> Pixels(out int width, out int height, out int stride)
    {
        uint w, h;
        nuint pitch;
        byte* pixels = NativeMethods.tt_image_pixels(raw.image, &w, &h, &pitch);
        // The sizes come from an image the child sent, so they are checked
        // before they bound a span.
        ulong length = h == 0 ? 0 : (ulong)pitch * (h - 1) + (ulong)w * 4;
        if (pixels == null || w == 0 || h == 0 || pitch < (ulong)w * 4 || pitch > int.MaxValue || length > int.MaxValue)
        {
            (width, height, stride) = (0, 0, 0);
            return default;
        }
        (width, height, stride) = ((int)w, (int)h, (int)pitch);
        return new ReadOnlySpan<byte>(pixels, (int)length);
    }
}

/// <summary>Rows <c>Start..End</c> of the new picture come from row <c>From</c> of the previous one.</summary>
[StructLayout(LayoutKind.Sequential)]
public readonly struct Scroll
{
    private readonly tt_scroll raw;

    public ushort Start => raw.start;

    public ushort End => raw.end;

    public ushort From => raw.from;
}

[Flags]
public enum StyleAttributes : ushort
{
    None = 0,
    Bold = NativeMethods.TT_ATTR_BOLD,
    Dim = NativeMethods.TT_ATTR_DIM,
    Italic = NativeMethods.TT_ATTR_ITALIC,
    Blink = NativeMethods.TT_ATTR_BLINK,
    Inverse = NativeMethods.TT_ATTR_INVERSE,
    Hidden = NativeMethods.TT_ATTR_HIDDEN,
    Strike = NativeMethods.TT_ATTR_STRIKE,
    Overline = NativeMethods.TT_ATTR_OVERLINE,
}

public enum Underline : byte
{
    None = NativeMethods.TT_UNDERLINE_NONE,
    SingleLine = NativeMethods.TT_UNDERLINE_SINGLE,
    DoubleLine = NativeMethods.TT_UNDERLINE_DOUBLE,
    Curly = NativeMethods.TT_UNDERLINE_CURLY,
    Dotted = NativeMethods.TT_UNDERLINE_DOTTED,
    Dashed = NativeMethods.TT_UNDERLINE_DASHED,
}
