namespace Scull.Core;

/// <summary>The lines of the new text that differ from the old one, both inclusive.</summary>
public readonly record struct TextChange(int FirstLine, int LastLine)
{
    /// <summary>
    /// What changed from <paramref name="previous"/> to <paramref name="next"/>, or null when
    /// nothing did, so a host raises <c>TextChanged</c> only for output that changed the
    /// text; a repaint of the same text, or a cursor blink, is not one. Without a
    /// previous text every line changed.
    /// </summary>
    public static TextChange? Between(ScreenText? previous, ScreenText next)
    {
        ArgumentNullException.ThrowIfNull(next);
        if (previous is null)
        {
            return new TextChange(0, next.LineCount - 1);
        }
        if (string.Equals(previous.Text, next.Text, StringComparison.Ordinal))
        {
            return null;
        }
        string[] old = previous.Text.Split('\n'), now = next.Text.Split('\n');
        int prefix = 0;
        while (prefix < old.Length && prefix < now.Length && old[prefix] == now[prefix])
        {
            prefix++;
        }
        int suffix = 0;
        while (suffix < old.Length - prefix && suffix < now.Length - prefix && old[^(suffix + 1)] == now[^(suffix + 1)])
        {
            suffix++;
        }
        // Lines only removed leave nothing in the new text to point at; the line where they were stands for them.
        int first = Math.Min(prefix, now.Length - 1);
        return new TextChange(first, Math.Max(first, now.Length - suffix - 1));
    }
}
