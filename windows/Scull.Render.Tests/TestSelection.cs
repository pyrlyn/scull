using System.Runtime.InteropServices;
using Scull.Core;
using Scull.Native;

namespace Scull.Render.Tests;

/// <summary>A selection made through the core's own calls, so a frame carries <c>TT_CELL_SELECTED</c>.</summary>
internal static class TestSelection
{
    [DllImport("scull_ffi", EntryPoint = "tt_term_select_start")]
    private static extern tt_status Start(nint term, uint kind, ushort row, ushort col);

    [DllImport("scull_ffi", EntryPoint = "tt_term_select_extend")]
    private static extern tt_status Extend(nint term, ushort row, ushort col);

    /// <summary>Selects cell by cell from one viewport cell to another, both included.</summary>
    public static void Select(Terminal terminal, ushort row, ushort col, ushort toRow, ushort toCol)
    {
        using Lease lease = terminal.LeaseTerm();
        Assert.AreEqual(tt_status.TT_OK, Start(lease.Pointer, NativeMethods.TT_SELECT_CELL, row, col));
        Assert.AreEqual(tt_status.TT_OK, Extend(lease.Pointer, toRow, toCol));
    }
}
