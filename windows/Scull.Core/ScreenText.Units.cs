namespace Scull.Core;

// The ITextRangeProvider moves, as pure functions of a range. Each count
// returned is the number of units actually moved, which UI Automation
// reports to the client when an edge of the text stops a move short.
public sealed partial class ScreenText
{
    // Benign race: two threads asking for the same unit both build the same array.
    private readonly int[]?[] bounds = new int[]?[7];

    private int[] Bounds(TextUnit unit)
    {
        if (!Enum.IsDefined(unit))
        {
            throw new ArgumentOutOfRangeException(nameof(unit), unit, "not a UI Automation text unit");
        }
        return bounds[(int)unit] ??= TextBoundaries.Of(Text, unit);
    }

    /// <summary>
    /// <c>ExpandToEnclosingUnit</c>: the start goes back to the start of its
    /// unit and the end forward to the end of its unit, so a range that is
    /// already whole units stays as it is. An empty range becomes the unit it is in.
    /// </summary>
    public TextRange ExpandToEnclosingUnit(TextRange range, TextUnit unit)
    {
        int[] b = Bounds(unit);
        if (Length == 0)
        {
            return new TextRange(0, 0);
        }
        (int start, int end) = Normalize(range);
        // At the very end the caret sits after the last unit, which is the one it expands to.
        int first = start == Length ? b.Length - 2 : Floor(b, start);
        int last = start == end ? first + 1 : Ceiling(b, end);
        return new TextRange(b[first], b[Math.Min(last, b.Length - 1)]);
    }

    /// <summary>
    /// <c>Move</c>: the range becomes one unit, <paramref name="count"/> units
    /// from the unit it expanded to, stopping at the text's edges. Returns the units moved.
    /// </summary>
    public TextRange Move(TextRange range, TextUnit unit, int count, out int moved)
    {
        TextRange expanded = ExpandToEnclosingUnit(range, unit);
        int[] b = Bounds(unit);
        if (Length == 0)
        {
            moved = 0;
            return expanded;
        }
        int from = Floor(b, expanded.Start);
        int to = (int)Math.Clamp((long)from + count, 0, b.Length - 2);
        moved = to - from;
        return new TextRange(b[to], b[to + 1]);
    }

    /// <summary>
    /// <c>MoveEndpointByUnit</c>: one end moves by whole units, from where it is
    /// to the unit boundaries around it. An end that crosses the other drags
    /// it along, leaving an empty range. Returns the units moved.
    /// </summary>
    public TextRange MoveEndpointByUnit(TextRange range, TextEndpoint endpoint, TextUnit unit, int count, out int moved)
    {
        int[] b = Bounds(unit);
        (int start, int end) = Normalize(range);
        int position = endpoint == TextEndpoint.Start ? start : end;
        moved = 0;
        for (; moved < count; moved++)
        {
            int next = Floor(b, position) + 1;
            if (next >= b.Length)
            {
                break;
            }
            position = b[next];
        }
        for (; moved > count; moved--)
        {
            int previous = Ceiling(b, position) - 1;
            if (previous < 0)
            {
                break;
            }
            position = b[previous];
        }
        return new TextRange(start, end).WithEndpoint(endpoint, position);
    }

    /// <summary>
    /// <c>FindText</c>: the first match of <paramref name="needle"/> inside <paramref name="within"/>
    /// (the last when <paramref name="backward"/>), or null. The match is by code unit, which
    /// is what a client's search string is.
    /// </summary>
    public TextRange? Find(TextRange within, string needle, bool backward, bool ignoreCase)
    {
        ArgumentNullException.ThrowIfNull(needle);
        (int start, int end) = Normalize(within);
        if (needle.Length == 0 || end - start < needle.Length)
        {
            return null;
        }
        StringComparison comparison = ignoreCase ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal;
        int at = backward
            ? Text.LastIndexOf(needle, end - 1, end - start, comparison)
            : Text.IndexOf(needle, start, end - start, comparison);
        return at < 0 ? null : new TextRange(at, at + needle.Length);
    }

    private (int Start, int End) Normalize(TextRange range)
    {
        int start = Math.Clamp(range.Start, 0, Length);
        return (start, Math.Clamp(range.End, start, Length));
    }

    // The index of the last boundary at or before the offset.
    private static int Floor(int[] b, int offset)
    {
        int at = Array.BinarySearch(b, offset);
        return at >= 0 ? at : ~at - 1;
    }

    // The index of the first boundary at or after the offset.
    private static int Ceiling(int[] b, int offset)
    {
        int at = Array.BinarySearch(b, offset);
        return at >= 0 ? at : ~at;
    }
}
