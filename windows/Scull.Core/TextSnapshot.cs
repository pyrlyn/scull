namespace Scull.Core;

/// <summary>A rectangle in pixels; <see cref="TextSnapshot.Rectangles"/> gives them in the surface's own.</summary>
public readonly record struct PixelRect(double X, double Y, double Width, double Height);

/// <summary>Where a pane's cells are: the cell size and the pane's top left, all in the surface's pixels.</summary>
public readonly record struct CellGeometry(double CellWidth, double CellHeight, double X, double Y)
{
    /// <summary>The rectangle of <paramref name="cells"/> cells (at least one) from a row and column, in the surface's pixels.</summary>
    public PixelRect Box(int row, int col, int cells) =>
        new(X + (col * CellWidth), Y + (row * CellHeight), Math.Max(cells, 1) * CellWidth, CellHeight);
}

/// <summary>
/// One pane's text as UI Automation sees it at one moment: the text, the cursor's
/// place in it, and the cell widths and geometry that turn offsets into pixels and
/// back. It is a copy, so a range a client holds stays valid while output goes on;
/// a client asks again for a fresh one. Pure, so it is tested without a window.
/// </summary>
public sealed class TextSnapshot
{
    private readonly int cols;
    private readonly byte[] widths;

    /// <param name="widths">The frame's cell widths, row-major, <paramref name="cols"/> to a row; a cell it lacks is one wide.</param>
    public TextSnapshot(ScreenText text, CellGeometry geometry, int cols, byte[] widths, int cursorRow, int cursorCol)
    {
        ArgumentNullException.ThrowIfNull(text);
        ArgumentNullException.ThrowIfNull(widths);
        (Text, Geometry, this.cols, this.widths) = (text, geometry, cols, widths);
        int caret = text.OffsetOf(cursorRow, cursorCol, WidthOf(cursorRow));
        Caret = new TextRange(caret, caret);
    }

    public ScreenText Text { get; }

    public CellGeometry Geometry { get; }

    /// <summary>The whole text.</summary>
    public TextRange Document => Text.Document;

    /// <summary>An empty range at the cursor.</summary>
    public TextRange Caret { get; }

    /// <summary>
    /// The pane as the last <see cref="Terminal.Update"/> left it. The frame is copied
    /// out because its cells are only valid until the next update.
    /// </summary>
    public static TextSnapshot Capture(Terminal terminal, CellGeometry geometry)
    {
        ArgumentNullException.ThrowIfNull(terminal);
        ScreenText text = ScreenText.Read(terminal);
        FrameView view = terminal.View;
        byte[] widths = new byte[view.Cells.Length];
        for (int i = 0; i < widths.Length; i++)
        {
            widths[i] = view.Cells[i].Width;
        }
        return new TextSnapshot(text, geometry, view.Cols, widths, view.Cursor.Row, view.Cursor.Col);
    }

    /// <summary>The cell width at a column of a row, for <see cref="ScreenText.Columns"/>.</summary>
    public Func<int, int> WidthOf(int row) => col =>
    {
        long at = ((long)row * cols) + col;
        return col >= 0 && col < cols && at >= 0 && at < widths.Length ? widths[at] : 1;
    };

    /// <summary>The cells <paramref name="range"/> covers as pixel rectangles, one per row.</summary>
    public IReadOnlyList<PixelRect> Rectangles(TextRange range) =>
    [
        .. Text.LineSpans(range, WidthOf).Select(span => new PixelRect(
            Geometry.X + (span.StartCol * Geometry.CellWidth),
            Geometry.Y + (span.Line * Geometry.CellHeight),
            (span.EndCol - span.StartCol) * Geometry.CellWidth,
            Geometry.CellHeight)),
    ];

    /// <summary>The character under a point in the surface's pixels; a point past the text is an empty range at its end.</summary>
    public TextRange RangeAt(double x, double y)
    {
        if (Geometry.CellWidth <= 0 || Geometry.CellHeight <= 0)
        {
            return new TextRange(Text.Length, Text.Length);
        }
        int row = (int)Math.Clamp(Math.Floor((y - Geometry.Y) / Geometry.CellHeight), 0, Text.LineCount - 1);
        int col = (int)Math.Clamp(Math.Floor((x - Geometry.X) / Geometry.CellWidth), 0, int.MaxValue);
        int at = Text.OffsetOf(row, col, WidthOf(row));
        return at >= Text.Length
            ? new TextRange(Text.Length, Text.Length)
            : Text.ExpandToEnclosingUnit(new TextRange(at, at), TextUnit.Character);
    }
}
