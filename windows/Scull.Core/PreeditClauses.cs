using System.Runtime.InteropServices;
using System.Text;
using Scull.Native;

namespace Scull.Core;

/// <summary>How an input method treats a stretch of its composing text; the same values as <c>TT_PREEDIT_*</c>.</summary>
public enum PreeditKind : byte
{
    /// <summary>Typed and not converted yet.</summary>
    Input = 0,

    /// <summary>Converted, but not the clause being edited.</summary>
    Converted = 1,

    /// <summary>The clause being converted now.</summary>
    Target = 2,
}

/// <summary>A stretch of the composing text as the input method reports it: UTF-16 offsets into the text, <see cref="End"/> exclusive.</summary>
public readonly record struct PreeditClause(int Start, int End, PreeditKind Kind);

/// <summary>A run of columns of the shown composing text with one kind. Same layout as <c>tt_preedit_span</c>.</summary>
[StructLayout(LayoutKind.Sequential)]
public readonly struct PreeditSpan
{
    private readonly tt_preedit_span raw;

    public ushort Col => raw.col;

    public ushort Cols => raw.cols;

    public PreeditKind Kind => (PreeditKind)raw.kind;
}

/// <summary>
/// How a renderer marks the composing text: pure over what the frame reports, so it is tested
/// without a window.
/// </summary>
public static class PreeditMarks
{
    /// <summary>
    /// What the cell at (<paramref name="row"/>, <paramref name="col"/>) is, or null when it is not
    /// composing text. Text set without clauses has no spans and is all <see cref="PreeditKind.Converted"/>:
    /// one plain line, as an input method that reports nothing gets.
    /// </summary>
    public static PreeditKind? At(Preedit preedit, ReadOnlySpan<PreeditSpan> spans, int row, int col)
    {
        if (preedit.Cols == 0 || row != preedit.Row || col < preedit.Col || col >= preedit.Col + preedit.Cols)
        {
            return null;
        }
        foreach (PreeditSpan span in spans)
        {
            if (col >= span.Col && col < span.Col + span.Cols)
            {
                return span.Kind;
            }
        }
        return PreeditKind.Converted;
    }

    /// <summary>
    /// The clauses as the core takes them, byte ranges of the UTF-8 text. An offset inside a surrogate
    /// pair moves back onto the pair's start, as the caret does; offsets past the text are cut to it.
    /// </summary>
    internal static tt_preedit_clause[] ToNative(string text, IReadOnlyList<PreeditClause> clauses)
    {
        var native = new tt_preedit_clause[clauses.Count];
        for (int i = 0; i < native.Length; i++)
        {
            PreeditClause c = clauses[i];
            native[i] = new tt_preedit_clause { start = ByteOffset(text, c.Start), end = ByteOffset(text, c.End), kind = (byte)c.Kind };
        }
        return native;
    }

    /// <summary>The byte offset in the UTF-8 form of <paramref name="text"/> of a UTF-16 <paramref name="index"/>, clamped to the text.</summary>
    internal static nuint ByteOffset(string text, int index)
    {
        index = Math.Clamp(index, 0, text.Length);
        if (index > 0 && index < text.Length && char.IsLowSurrogate(text[index]) && char.IsHighSurrogate(text[index - 1]))
        {
            index--;
        }
        return (nuint)Encoding.UTF8.GetByteCount(text.AsSpan(0, index));
    }
}
