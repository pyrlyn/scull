namespace Scull.Core;

/// <summary>A rectangle in pixels; <see cref="TextSnapshot.Rectangles"/> gives them in the surface's own.</summary>
public readonly record struct PixelRect(double X, double Y, double Width, double Height);

/// <summary>Where a pane's cells are: the cell size and the pane's top left, all in the surface's pixels.</summary>
public readonly record struct CellGeometry(double CellWidth, double CellHeight, double X, double Y)
{
    /// <summary>The rectangle of <paramref name="cells"/> cells (at least one) from a row and column, in the surface's pixels.</summary>
    public PixelRect Box(int row, int col, int cells) =>
        new(X + (col * CellWidth), Y + (row * CellHeight), Math.Max(cells, 1) * CellWidth, CellHeight);

    /// <summary>Where an input method's candidate window goes: under the composing text, or at the cursor when there is none.</summary>
    public PixelRect CompositionBox(Preedit preedit, Cursor cursor) =>
        preedit.Cols > 0 ? Box(preedit.Row, preedit.Col, preedit.Cols) : Box(cursor.Row, cursor.Col, 1);
}

/// <summary>Cells from the first to the last of a run of text, both ends included: the viewport's rows and columns.</summary>
public readonly record struct CellSpan(int StartRow, int StartCol, int EndRow, int EndCol);

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
    private readonly ushort[] cellStyles;
    private readonly Style[] styles;

    /// <param name="widths">The frame's cell widths, row-major, <paramref name="cols"/> to a row; a cell it lacks is one wide.</param>
    /// <param name="selected">The first to the last selected cell of the frame; null when none is.</param>
    /// <param name="cellStyles">The frame's style index of each cell, laid out like <paramref name="widths"/>; none when the cells have no styles to tell.</param>
    /// <param name="styles">The style table <paramref name="cellStyles"/> index.</param>
    /// <param name="look">What a client needs to read the styles as text attributes; null when the host gave none.</param>
    public TextSnapshot(ScreenText text, CellGeometry geometry, int cols, byte[] widths, int cursorRow, int cursorCol, CellSpan? selected = null,
        ushort[]? cellStyles = null, Style[]? styles = null, TextLook? look = null)
    {
        ArgumentNullException.ThrowIfNull(text);
        ArgumentNullException.ThrowIfNull(widths);
        (Text, Geometry, this.cols, this.widths) = (text, geometry, cols, widths);
        (this.cellStyles, this.styles) = (cellStyles ?? [], styles ?? []);
        Attributes = look is null ? null : new TextAttributes(this, look);
        int caret = text.OffsetOf(cursorRow, cursorCol, WidthOf(cursorRow));
        Caret = new TextRange(caret, caret);
        Selection = selected is { } cells ? RangeOf(cells) : null;
    }

    public ScreenText Text { get; }

    public CellGeometry Geometry { get; }

    /// <summary>The whole text.</summary>
    public TextRange Document => Text.Document;

    /// <summary>The cells' style as UI Automation text attributes; null when the host gave no <see cref="TextLook"/>.</summary>
    public TextAttributes? Attributes { get; }

    /// <summary>The style of the cell at a row and column; the default one when the snapshot has none for it.</summary>
    public Style StyleAt(int row, int col)
    {
        long at = ((long)row * cols) + col;
        bool inside = row >= 0 && col >= 0 && col < cols && at < cellStyles.Length;
        return inside && cellStyles[at] < styles.Length ? styles[cellStyles[at]] : default;
    }

    /// <summary>An empty range at the cursor.</summary>
    public TextRange Caret { get; }

    /// <summary>
    /// The selection as one range from its first to its last cell, or null with none. A block
    /// selection is one range too, so the cells between its columns that it left out are inside it.
    /// </summary>
    public TextRange? Selection { get; }

    /// <summary>
    /// The pane as the last <see cref="Terminal.Update"/> left it. The frame is copied
    /// out because its cells are only valid until the next update.
    /// </summary>
    public static TextSnapshot Capture(Terminal terminal, CellGeometry geometry, TextLook? look = null)
    {
        ArgumentNullException.ThrowIfNull(terminal);
        ScreenText text = ScreenText.Read(terminal);
        FrameView view = terminal.View;
        byte[] widths = new byte[view.Cells.Length];
        ushort[] cellStyles = new ushort[widths.Length];
        for (int i = 0; i < widths.Length; i++)
        {
            widths[i] = view.Cells[i].Width;
            cellStyles[i] = view.Cells[i].Style;
        }
        return new TextSnapshot(text, geometry, view.Cols, widths, view.Cursor.Row, view.Cursor.Col, SelectedCells(view), cellStyles, view.Styles.ToArray(), look);
    }

    private static CellSpan? SelectedCells(FrameView view)
    {
        ReadOnlySpan<Cell> cells = view.Cells;
        int first = -1, last = -1;
        for (int i = 0; i < cells.Length; i++)
        {
            if (cells[i].IsSelected)
            {
                first = first < 0 ? i : first;
                last = i;
            }
        }
        if (first < 0 || view.Cols <= 0)
        {
            return null;
        }
        return new CellSpan(first / view.Cols, first % view.Cols, last / view.Cols, last % view.Cols);
    }

    // The text from the character at the first cell to the end of the character at the last.
    private TextRange RangeOf(CellSpan cells)
    {
        int start = Text.OffsetOf(cells.StartRow, cells.StartCol, WidthOf(cells.StartRow));
        var stops = Text.Columns(cells.EndRow, WidthOf(cells.EndRow));
        int end = stops.Count == 0 ? Text.Length : stops.Where(x => x.Col > cells.EndCol).Select(x => x.Index).DefaultIfEmpty(stops[^1].Index).First();
        return new TextRange(start, Math.Max(start, end));
    }

    /// <summary>
    /// The cells <paramref name="range"/> covers, for selecting it through the core; null for a range with
    /// no character (empty, or only newlines). A wide character's end is its last column.
    /// </summary>
    public CellSpan? CellsOf(TextRange range)
    {
        int start = Math.Clamp(range.Start, 0, Text.Length), end = Math.Clamp(range.End, start, Text.Length);
        // The newline ending a row belongs to the row, and has no cell of its own.
        while (start < end && Text.Text[start] == '\n')
        {
            start++;
        }
        while (end > start && Text.Text[end - 1] == '\n')
        {
            end--;
        }
        if (start >= end)
        {
            return null;
        }
        (int startRow, int startCol) = CellAt(start);
        (int endRow, int endCol) = CellAt(end - 1);
        return new CellSpan(startRow, startCol, endRow, endCol + Math.Max(1, WidthOf(endRow)(endCol)) - 1);
    }

    // The row and first column of the character holding an offset.
    private (int Row, int Col) CellAt(int offset)
    {
        int row = Text.LineFor(offset);
        var stops = Text.Columns(row, WidthOf(row));
        return (row, stops.Last(x => x.Index <= offset).Col);
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
