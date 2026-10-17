namespace Scull.Core;

/// <summary>
/// What a client needs to read a pane's cell styles as text attributes: the palette that
/// turns a style into the two colours on screen, and the font the pane shows.
/// </summary>
/// <param name="Colors">A style's foreground and background as 0xRRGGBB, reverse video applied.</param>
/// <param name="FontName">The family the pane is set to.</param>
/// <param name="FontPoints">The size in points.</param>
public sealed record TextLook(Func<Style, (uint Foreground, uint Background)> Colors, string FontName, double FontPoints);

/// <summary>The UI Automation text attribute ids the terminal answers (<c>UIA_*AttributeId</c> in UIAutomationClient.h).</summary>
public static class AttributeId
{
    public const int BackgroundColor = 40001;
    public const int FontName = 40005;
    public const int FontSize = 40006;
    public const int FontWeight = 40007;
    public const int ForegroundColor = 40008;
    public const int IsHidden = 40013;
    public const int IsItalic = 40014;
    public const int IsReadOnly = 40015;
    public const int OverlineStyle = 40024;
    public const int StrikethroughStyle = 40026;
    public const int UnderlineStyle = 40030;
}

/// <summary>How a text range answers one attribute.</summary>
public enum AttributeState
{
    /// <summary>The terminal does not report this attribute.</summary>
    Unsupported,

    /// <summary>The cells of the range differ in it.</summary>
    Mixed,

    /// <summary>All the cells agree; <see cref="AttributeResult.Value"/> is the answer.</summary>
    Value,
}

/// <summary>An attribute's answer. The value is a boxed int (colours as 0x00BBGGRR, weights, line styles), double, bool or string, as UI Automation takes it.</summary>
public readonly record struct AttributeResult(AttributeState State, object? Value = null)
{
    public static AttributeResult Unsupported { get; } = new(AttributeState.Unsupported);

    public static AttributeResult Mixed { get; } = new(AttributeState.Mixed);
}

/// <summary>
/// The cells of a <see cref="TextSnapshot"/> read as UI Automation text attributes: the pane's
/// font, the colours a cell shows, bold, italic, the line styles, hidden and read-only. A range
/// whose characters differ answers mixed, and <see cref="Find"/> looks for a run of one value.
/// Pure over the snapshot, so it is tested without a window.
/// </summary>
public sealed class TextAttributes(TextSnapshot snapshot, TextLook look)
{
    // TextDecorationLineStyle in UIAutomationTypes.h.
    private const int LineNone = 0, LineSingle = 1, LineDouble = 3, LineDot = 4, LineDash = 5, LineWavy = 8;
    private const int WeightNormal = 400, WeightBold = 700;

    /// <summary>The attribute over <paramref name="range"/>: the style of the character at an empty range, else of every character it touches.</summary>
    public AttributeResult Get(TextRange range, int attribute)
    {
        if (!IsSupported(attribute))
        {
            return AttributeResult.Unsupported;
        }
        object? first = null;
        bool any = false;
        foreach ((_, _, Style style) in Characters(range))
        {
            object value = Of(style, attribute);
            if (!any)
            {
                (first, any) = (value, true);
            }
            else if (!Same(first!, value))
            {
                return AttributeResult.Mixed;
            }
        }
        return new AttributeResult(AttributeState.Value, any ? first : Of(default, attribute));
    }

    /// <summary>
    /// The first run, from the start of <paramref name="range"/> or backward from its end, whose
    /// characters all have <paramref name="value"/> for the attribute, cut to the range; null for
    /// none, for an empty range and for an attribute the terminal does not report.
    /// </summary>
    public TextRange? Find(TextRange range, int attribute, object value, bool backward)
    {
        ArgumentNullException.ThrowIfNull(value);
        if (!IsSupported(attribute))
        {
            return null;
        }
        List<(int Start, int End, Style Style)> characters = [.. Characters(range, nonEmpty: true)];
        bool Matches(int i) => Same(Of(characters[i].Style, attribute), value);
        int at = backward ? Enumerable.Range(0, characters.Count).Reverse().FirstOrDefault(Matches, -1) : Enumerable.Range(0, characters.Count).FirstOrDefault(Matches, -1);
        if (at < 0)
        {
            return null;
        }
        int from = at, to = at;
        while (from > 0 && Matches(from - 1))
        {
            from--;
        }
        while (to + 1 < characters.Count && Matches(to + 1))
        {
            to++;
        }
        return new TextRange(characters[from].Start, characters[to].End);
    }

    private static bool IsSupported(int attribute) => attribute is AttributeId.BackgroundColor or AttributeId.FontName
        or AttributeId.FontSize or AttributeId.FontWeight or AttributeId.ForegroundColor or AttributeId.IsHidden
        or AttributeId.IsItalic or AttributeId.IsReadOnly or AttributeId.OverlineStyle or AttributeId.StrikethroughStyle
        or AttributeId.UnderlineStyle;

    private object Of(Style style, int attribute)
    {
        StyleAttributes attributes = style.Attributes;
        return attribute switch
        {
            AttributeId.ForegroundColor => ColorRef(look.Colors(style).Foreground),
            AttributeId.BackgroundColor => ColorRef(look.Colors(style).Background),
            AttributeId.FontName => look.FontName,
            AttributeId.FontSize => look.FontPoints,
            AttributeId.FontWeight => (attributes & StyleAttributes.Bold) != 0 ? WeightBold : WeightNormal,
            AttributeId.IsItalic => (attributes & StyleAttributes.Italic) != 0,
            AttributeId.IsHidden => (attributes & StyleAttributes.Hidden) != 0,
            // The pane is a view of the program's screen: a client cannot type into it as text.
            AttributeId.IsReadOnly => false,
            AttributeId.StrikethroughStyle => (attributes & StyleAttributes.Strike) != 0 ? LineSingle : LineNone,
            AttributeId.OverlineStyle => (attributes & StyleAttributes.Overline) != 0 ? LineSingle : LineNone,
            _ => style.Underline switch
            {
                Underline.SingleLine => LineSingle,
                Underline.DoubleLine => LineDouble,
                Underline.Curly => LineWavy,
                Underline.Dotted => LineDot,
                Underline.Dashed => LineDash,
                _ => LineNone,
            },
        };
    }

    // A COLORREF: 0x00BBGGRR, as UI Automation hands colours around.
    private static int ColorRef(uint rgb) => (int)(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF));

    // A client's value is whatever number type it boxed.
    private static bool Same(object a, object b) =>
        a.Equals(b) || (Number(a) is { } x && Number(b) is { } y && x == y);

    private static double? Number(object value) => value switch
    {
        int i => i,
        uint u => u,
        long l => l,
        short s => s,
        byte b => b,
        float f => f,
        double d => d,
        _ => null,
    };

    // The characters the range touches with their text offsets, or for an empty range the one it sits on.
    private List<(int Start, int End, Style Style)> Characters(TextRange range, bool nonEmpty = false)
    {
        ScreenText text = snapshot.Text;
        int start = Math.Clamp(range.Start, 0, text.Length), end = Math.Clamp(range.End, start, text.Length);
        List<(int, int, Style)> found = [];
        bool empty = start == end;
        if (empty && nonEmpty)
        {
            return found;
        }
        // The newline that ends a range belongs to the row before it.
        for (int line = text.LineFor(start), last = text.LineFor(Math.Max(start, end - 1)); line <= last; line++)
        {
            var stops = text.Columns(line, snapshot.WidthOf(line));
            for (int i = 0; i + 1 < stops.Count; i++)
            {
                (int from, int col) = stops[i];
                int to = stops[i + 1].Index;
                bool touched = empty ? from <= start && (start < to || i + 2 == stops.Count) : from < end && to > start;
                if (touched)
                {
                    found.Add((from, to, snapshot.StyleAt(line, col)));
                }
            }
        }
        return found;
    }
}
