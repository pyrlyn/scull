using System.Globalization;

namespace Scull.Core;

/// <summary>The cells <see cref="ScreenText.LineSpans"/> found on one row for a range.</summary>
public readonly record struct LineSpan(int Line, int StartCol, int EndCol);

/// <summary>
/// The screen's text as UI Automation clients index it: one string, a line per
/// row, offsets in UTF-16. The geometry is the frame's cells; this only maps
/// between offsets, lines and columns, and moves ranges by text unit. It is
/// pure, so it needs neither a window nor a provider, and it is immutable.
/// </summary>
public sealed partial class ScreenText
{
    // The UTF-16 offset each line starts at; never empty. A text ending in a
    // newline ends in an empty line.
    private readonly int[] starts;

    public ScreenText(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        Text = text;
        var lines = new List<int> { 0 };
        for (int i = text.IndexOf('\n', StringComparison.Ordinal); i >= 0; i = text.IndexOf('\n', i + 1))
        {
            lines.Add(i + 1);
        }
        starts = [.. lines];
    }

    /// <summary>The viewport as the core's <c>tt_term_read_text</c> gives it.</summary>
    public static ScreenText Read(Terminal terminal)
    {
        ArgumentNullException.ThrowIfNull(terminal);
        return new ScreenText(terminal.ReadText());
    }

    public string Text { get; }

    public int Length => Text.Length;

    public int LineCount => starts.Length;

    /// <summary>The whole text.</summary>
    public TextRange Document => new(0, Length);

    /// <summary>The line holding <paramref name="offset"/>; past the end it is the last line.</summary>
    public int LineFor(int offset)
    {
        int at = Array.BinarySearch(starts, Math.Max(0, offset));
        return at >= 0 ? at : ~at - 1;
    }

    /// <summary>The line's text with its newline, as a text pattern answers it; null for a line that does not exist.</summary>
    public TextRange? RangeForLine(int line)
    {
        if ((uint)line >= (uint)starts.Length)
        {
            return null;
        }
        return new TextRange(starts[line], line + 1 < starts.Length ? starts[line + 1] : Length);
    }

    /// <summary>
    /// The text of <paramref name="range"/> clamped to the text, cut to
    /// <paramref name="maxLength"/> UTF-16 units unless that is negative.
    /// </summary>
    public string GetText(TextRange range, int maxLength = -1)
    {
        int start = Math.Clamp(range.Start, 0, Length), end = Math.Clamp(range.End, start, Length);
        if (maxLength >= 0)
        {
            end = Math.Min(end, (int)Math.Min((long)start + maxLength, int.MaxValue));
        }
        return Text[start..end];
    }

    /// <summary>
    /// Where each character of <paramref name="line"/> starts, as (UTF-16 offset,
    /// column), then the line's end. <paramref name="width"/> answers the row's cell
    /// width at a column, so a wide character takes its columns.
    /// </summary>
    public IReadOnlyList<(int Index, int Col)> Columns(int line, Func<int, int> width)
    {
        ArgumentNullException.ThrowIfNull(width);
        if (RangeForLine(line) is not { } range)
        {
            return [];
        }
        int end = range.End - (Text.AsSpan(range.Start, range.Length).EndsWith("\n") ? 1 : 0);
        string row = Text[range.Start..end];
        var stops = new List<(int, int)>();
        int col = 0;
        foreach (int at in StringInfo.ParseCombiningCharacters(row))
        {
            stops.Add((range.Start + at, col));
            col += Math.Max(1, width(col));
        }
        stops.Add((end, col));
        return stops;
    }

    /// <summary>The offset of the character at <paramref name="col"/> of <paramref name="line"/>, or the line's end; the cursor's offset or a hit test.</summary>
    public int OffsetOf(int line, int col, Func<int, int> width)
    {
        var stops = Columns(line, width);
        for (int i = stops.Count - 1; i >= 0; i--)
        {
            if (stops[i].Col <= col)
            {
                return stops[i].Index;
            }
        }
        return Length;
    }

    /// <summary>
    /// The cells <paramref name="range"/> covers, one span per row, for
    /// bounding rectangles: the first and last row partly, the rows between whole.
    /// A span is at least one cell wide, so an empty range or a bare newline still has a place.
    /// <paramref name="widthOfRow"/> answers, for a row, the cell width at a column.
    /// </summary>
    public IReadOnlyList<LineSpan> LineSpans(TextRange range, Func<int, Func<int, int>> widthOfRow)
    {
        ArgumentNullException.ThrowIfNull(widthOfRow);
        int start = Math.Clamp(range.Start, 0, Length), end = Math.Clamp(range.End, start, Length);
        var spans = new List<LineSpan>();
        // The newline that ends a range belongs to the row before it.
        for (int line = LineFor(start), last = LineFor(Math.Max(start, end - 1)); line <= last; line++)
        {
            var stops = Columns(line, widthOfRow(line));
            int from = stops[0].Index, to = stops[^1].Index;
            int s = Math.Clamp(start, from, to), e = Math.Clamp(end, from, to);
            int startCol = stops.LastOrDefault(x => x.Index <= s).Col;
            int endCol = stops.Where(x => x.Index >= e).Select(x => x.Col).DefaultIfEmpty(stops[^1].Col).First();
            spans.Add(new LineSpan(line, startCol, Math.Max(endCol, startCol + 1)));
        }
        return spans;
    }
}
