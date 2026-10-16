using System.Globalization;
using System.Text;

namespace Scull.Core;

/// <summary>A <c>TextUnit</c> of UI Automation; the values are <c>UIA_TextUnit</c>'s.</summary>
public enum TextUnit
{
    Character = 0,
    Format = 1,
    Word = 2,
    Line = 3,
    Paragraph = 4,
    Page = 5,
    Document = 6,
}

/// <summary>Which end of a <see cref="TextRange"/> moves; the values are <c>TextPatternRangeEndpoint</c>'s.</summary>
public enum TextEndpoint
{
    Start = 0,
    End = 1,
}

/// <summary>UTF-16 offsets <c>[Start, End)</c> into a <see cref="ScreenText"/>; empty when they are equal.</summary>
public readonly record struct TextRange(int Start, int End)
{
    public int Length => End - Start;

    public bool IsEmpty => End == Start;
}

/// <summary>Where each <see cref="TextUnit"/> starts in a text.</summary>
internal static class TextBoundaries
{
    /// <summary>
    /// The sorted offsets at which units start, then the text's length, so unit
    /// <c>i</c> is <c>[b[i], b[i + 1])</c>. An empty text has the single entry 0.
    /// </summary>
    internal static int[] Of(string text, TextUnit unit) => unit switch
    {
        TextUnit.Character => Characters(text),
        TextUnit.Word => Words(text),
        TextUnit.Line or TextUnit.Paragraph => Lines(text),
        // A terminal has no formatting runs and shows one page, so the rest are the whole text.
        TextUnit.Format or TextUnit.Page or TextUnit.Document => text.Length == 0 ? [0] : [0, text.Length],
        _ => throw new ArgumentOutOfRangeException(nameof(unit), unit, "not a UI Automation text unit"),
    };

    private static int[] Characters(string text) => [.. StringInfo.ParseCombiningCharacters(text), text.Length];

    private static int[] Lines(string text)
    {
        var b = new List<int> { 0 };
        for (int i = text.IndexOf('\n', StringComparison.Ordinal); i >= 0 && i + 1 < text.Length;)
        {
            b.Add(i + 1);
            i = text.IndexOf('\n', i + 1);
        }
        // A text that ends in a newline ends in an empty line, which has no unit of its own.
        if (text.Length > 0)
        {
            b.Add(text.Length);
        }
        return [.. b];
    }

    // A word is a run of letters and digits, or of anything else that is not
    // blank, with the blanks after it. A newline and each ideograph or kana
    // stand alone: such scripts have no spaces to break on, and a screen
    // reader that read a whole row as one word would be of no use.
    private static int[] Words(string text)
    {
        var b = new List<int> { 0 };
        for (int i = 0; i < text.Length;)
        {
            Rune first = RuneAt(text, i);
            if (first.Value == '\n')
            {
                i++;
            }
            else if (IsBlank(first))
            {
                i = Skip(text, i, IsBlank);
            }
            else
            {
                i += first.Utf16SequenceLength;
                if (!IsSyllabary(first))
                {
                    bool letters = IsLetter(first);
                    i = Skip(text, i, r => !IsBlank(r) && r.Value != '\n' && !IsSyllabary(r) && IsLetter(r) == letters);
                }
                i = Skip(text, i, IsBlank);
            }
            b.Add(i);
        }
        return [.. b];
    }

    private static int Skip(string text, int at, Func<Rune, bool> belongs)
    {
        while (at < text.Length)
        {
            Rune rune = RuneAt(text, at);
            if (!belongs(rune))
            {
                break;
            }
            at += rune.Utf16SequenceLength;
        }
        return at;
    }

    // A lone surrogate is one replacement character wide, as the text decoder makes it.
    private static Rune RuneAt(string text, int at) => Rune.TryGetRuneAt(text, at, out Rune rune) ? rune : Rune.ReplacementChar;

    private static bool IsBlank(Rune rune) => rune.Value != '\n' && Rune.IsWhiteSpace(rune);

    private static bool IsLetter(Rune rune) =>
        Rune.IsLetterOrDigit(rune) || rune.Value == '_'
        || Rune.GetUnicodeCategory(rune) is UnicodeCategory.NonSpacingMark or UnicodeCategory.SpacingCombiningMark or UnicodeCategory.EnclosingMark;

    // Hiragana and katakana, CJK ideographs and their extensions and compatibility forms.
    private static bool IsSyllabary(Rune rune) => rune.Value is
        (>= 0x3040 and <= 0x30FF) or (>= 0x3400 and <= 0x4DBF) or (>= 0x4E00 and <= 0x9FFF)
        or (>= 0xF900 and <= 0xFAFF) or (>= 0x20000 and <= 0x3FFFF);
}
