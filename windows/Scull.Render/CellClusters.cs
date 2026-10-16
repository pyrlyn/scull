using System.Buffers;
using System.Globalization;
using System.Text;
using Scull.Core;

namespace Scull.Render;

/// <summary>
/// The code points a cluster cell holds after its first, which the renderer
/// draws over the base cell until runs are shaped (T17.5.2). A cell carries
/// only its first code point; the whole cluster is in its run's text, one
/// piece per cell, so the pieces are split again by grapheme. Buffers are
/// reused, so a row of clusters seen before allocates nothing.
/// </summary>
internal sealed class CellClusters
{
    private char[] chars = new char[256];
    private int[] marks = new int[64];
    private int[] start = [], count = [];
    private int total;

    /// <summary>Splits <paramref name="line"/>'s run text over <paramref name="cells"/>, the cells of the same row.</summary>
    public void Load(Row line, ReadOnlySpan<Cell> cells)
    {
        if (start.Length < cells.Length)
        {
            (start, count) = (new int[cells.Length], new int[cells.Length]);
        }
        start.AsSpan(0, cells.Length).Clear();
        count.AsSpan(0, cells.Length).Clear();
        total = 0;
        foreach (Run run in line.Runs)
        {
            ReadOnlySpan<byte> text = line.TextOf(run);
            // UTF-16 never needs more units than UTF-8 needs bytes.
            if (chars.Length < text.Length)
            {
                chars = new char[text.Length];
            }
            ReadOnlySpan<char> rest = chars.AsSpan(0, Encoding.UTF8.GetChars(text, chars));
            int end = Math.Min(run.Col + run.Cols, cells.Length);
            for (int col = run.Col; col < end && !rest.IsEmpty; col += Math.Max(1, (int)run.Width))
            {
                Cell cell = cells[col];
                // A piece that does not open with its cell's code point means
                // the core and .NET split graphemes differently; the rest of
                // the run then draws no marks rather than the wrong ones.
                if (Rune.DecodeFromUtf16(rest, out Rune first, out int used) != OperationStatus.Done || (uint)first.Value != cell.CodePoint)
                {
                    break;
                }
                int length = cell.IsCluster ? StringInfo.GetNextTextElementLength(rest) : used;
                start[col] = total;
                for (int i = used; i < length;)
                {
                    Rune.DecodeFromUtf16(rest[i..length], out Rune mark, out int size);
                    if (total == marks.Length)
                    {
                        Array.Resize(ref marks, total * 2);
                    }
                    marks[total++] = mark.Value;
                    i += Math.Max(1, size);
                }
                count[col] = total - start[col];
                rest = rest[length..];
            }
        }
    }

    /// <summary>The code points after the first in the cluster at <paramref name="col"/> of the last loaded row.</summary>
    public ReadOnlySpan<int> At(int col) => col < start.Length ? marks.AsSpan(start[col], count[col]) : default;

    /// <summary>Whether <paramref name="codePoint"/> is drawn over its base: Unicode's marks only, so joiners and selectors wait for shaping.</summary>
    public static bool IsMark(int codePoint) => CharUnicodeInfo.GetUnicodeCategory(codePoint)
        is UnicodeCategory.NonSpacingMark or UnicodeCategory.SpacingCombiningMark or UnicodeCategory.EnclosingMark;

    /// <summary>
    /// The pen of a mark over a base <paramref name="span"/> pixels wide at
    /// <paramref name="x"/>, its ink <paramref name="inkLeft"/> off the pen and
    /// <paramref name="inkWidth"/> wide. Ink centred left of the pen was made
    /// to follow the base's advance, as zero-width marks are; ink right of it
    /// was made to sit over the base in place, as some monospaced fonts do.
    /// </summary>
    public static int Pen(int x, int span, int inkLeft, int inkWidth) => inkLeft * 2 + inkWidth < 0 ? x + span : x;
}
