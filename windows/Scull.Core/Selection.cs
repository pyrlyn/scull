using Scull.Native;

namespace Scull.Core;

// Mouse selection in the core's vocabulary, from the plain numbers WinUI reports, so it is tested
// on every runner without WinUI. It mirrors macos/Sources/ScullKit/SelectionGesture.swift and the
// drag state of TerminalView+Selection.swift; the core keeps the selection and marks it in the frame.

/// <summary>How a selection starts; the values are the core's <c>TT_SELECT_*</c>.</summary>
public enum SelectionKind : uint
{
    Cell = NativeMethods.TT_SELECT_CELL,
    Word = NativeMethods.TT_SELECT_WORD,
    Line = NativeMethods.TT_SELECT_LINE,
    Block = NativeMethods.TT_SELECT_BLOCK,
}

/// <summary>What a left press does to the selection.</summary>
/// <param name="Extend">Grow the selection that is there; otherwise start one of <paramref name="Kind"/>.</param>
public readonly record struct SelectionPress(bool Extend, SelectionKind Kind)
{
    /// <summary>A single click that never moved selects nothing when it is released.</summary>
    public bool ClearsOnRelease => !Extend && Kind is SelectionKind.Cell or SelectionKind.Block;
}

public static class SelectionGesture
{
    /// <summary>Shift asks for the host's selection even while the program tracks the mouse, as in xterm.</summary>
    public static bool ForcesSelection(KeyModifiers held) => (held & KeyModifiers.Shift) != 0;

    /// <summary>
    /// Two clicks take a word, three or more a line; Alt (not AltGr) drags a block. Shift grows
    /// a selection that is there, or starts one when none is.
    /// </summary>
    public static SelectionPress Press(int clickCount, KeyModifiers held, bool hasSelection)
    {
        if (ForcesSelection(held) && hasSelection)
        {
            return new SelectionPress(true, SelectionKind.Cell);
        }
        return clickCount switch
        {
            2 => new SelectionPress(false, SelectionKind.Word),
            >= 3 => new SelectionPress(false, SelectionKind.Line),
            _ => new SelectionPress(false, (held & (KeyModifiers.Alt | KeyModifiers.Control)) == KeyModifiers.Alt ? SelectionKind.Block : SelectionKind.Cell),
        };
    }
}

/// <summary>
/// WinUI reports no click count, so presses count up while each comes within the system's
/// double-click time and distance of the one before.
/// </summary>
public sealed class ClickCounter
{
    private long lastMs;
    private double lastX, lastY;
    private int count;

    /// <summary>The click count of a press at <paramref name="x"/>, <paramref name="y"/> pixels, <paramref name="ms"/> on any clock that only runs forward.</summary>
    /// <param name="intervalMs">The longest gap between clicks of a series.</param>
    /// <param name="slop">The farthest the pointer may stray from the last click, in pixels on each axis.</param>
    public int Press(long ms, double x, double y, long intervalMs, double slop)
    {
        bool again = count > 0 && ms >= lastMs && ms - lastMs <= intervalMs && Math.Abs(x - lastX) <= slop && Math.Abs(y - lastY) <= slop;
        (lastMs, lastX, lastY) = (ms, x, y);
        return count = again ? count + 1 : 1;
    }
}

/// <summary>A left-button gesture the host took for selection, from press to release.</summary>
public sealed class SelectionDrag(uint col, uint row, bool clearsOnRelease)
{
    private (uint Col, uint Row) cell = (col, row);

    /// <summary>A single click that never moved selects nothing when released.</summary>
    public bool ClearsOnRelease { get; private set; } = clearsOnRelease;

    /// <summary>
    /// The pointer moved to a cell. True when the selection's free end follows: the cell changed, or the
    /// view scrolled (the selection is anchored to the history, so it grows with it).
    /// </summary>
    public bool Move(uint col, uint row, bool scrolled)
    {
        if (!scrolled && cell == (col, row))
        {
            return false;
        }
        cell = (col, row);
        ClearsOnRelease = false;
        return true;
    }

    /// <summary>Which way a point <paramref name="y"/> pixels below the text area's top edge scrolls the history: one row back above it, one towards the screen below it, per motion event.</summary>
    public static int EdgeScroll(double y, double height) => y < 0 ? 1 : y >= height ? -1 : 0;
}
