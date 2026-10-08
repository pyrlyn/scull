using System.Runtime.InteropServices;
using System.Text;
using Scull.Native;

namespace Scull.Core.Tests;

[TestClass]
public sealed unsafe class TerminalTests
{
    private static int frees;

    // Built with `--features test-hooks` (`just windows-core-test`); not in
    // the generated bindings because no shipped library has it.
    [DllImport("scull_ffi", EntryPoint = "tt_term_test_panic")]
    private static extern tt_status TestPanic(nint term);

    [TestMethod]
    public void TheLibrarySpeaksTheAbiTheBindingsWereBuiltFor()
    {
        Assert.AreEqual(Abi.HostVersion, Abi.LibraryVersion);
        Abi.EnsureCompatible();
    }

    [TestMethod]
    [DataRow(0x0005u, 0x0005u, true)]
    [DataRow(0x0005u, 0x0006u, false)] // 0.x: every minor may break
    [DataRow(0x0006u, 0x0005u, false)]
    [DataRow(0x0005u, 0x1_0005u, false)] // another major
    [DataRow(0x1_0002u, 0x1_0003u, true)] // a newer minor only adds
    [DataRow(0x1_0003u, 0x1_0002u, false)]
    public void AbiCompatibility(uint host, uint library, bool compatible) =>
        Assert.AreEqual(compatible, Abi.Compatible(host, library));

    [TestMethod]
    public void StatusMirrorsEveryCoreStatus()
    {
        string[] core = [.. Enum.GetValues<tt_status>().Select(s => $"{(int)s}:{s.ToString()[3..].Replace("_", "", StringComparison.Ordinal)}")];
        string[] ours = [.. Enum.GetValues<Status>().Select(s => $"{(int)s}:{s.ToString().ToUpperInvariant()}")];
        CollectionAssert.AreEqual(core, ours);
    }

    [TestMethod]
    public void FedTextReadsBackAsTextAndAsAFrame()
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed("hello\r\nwörld"u8);

        Assert.AreEqual("hello\nwörld", terminal.ReadText(0, 2));
        Assert.AreEqual("wörld", terminal.ReadText(1, 1));

        Assert.IsTrue(terminal.Update());
        FrameView view = terminal.View;
        Assert.AreEqual((20, 4), (view.Cols, view.Rows));
        Assert.IsTrue(view.Full);
        Assert.AreEqual(20 * 4, view.Cells.Length);
        Assert.AreEqual('h', (char)view.Cells[0].CodePoint);
        Assert.AreEqual(1, view.Cells[0].Width);
        Assert.AreEqual(new Cursor(1, 5, true), view.Cursor);

        Row first = view.Lines[0];
        Assert.AreEqual(1, first.Runs.Length);
        Assert.AreEqual("hello", Encoding.UTF8.GetString(first.TextOf(first.Runs[0])));
        Assert.AreEqual(StyleAttributes.None, view.StyleAt(first.Runs[0].Style).Attributes);

        Assert.IsTrue(terminal.Update());
        Assert.IsFalse(terminal.View.Full);
        Assert.AreEqual(0, terminal.View.Dirty.Length, "nothing changed since the last update");
    }

    [TestMethod]
    public void StylesComeThroughTheTable()
    {
        using var terminal = new Terminal(10, 2);
        terminal.Feed("\e[1;4mB"u8);
        terminal.Update();
        FrameView view = terminal.View;
        Style style = view.StyleAt(view.Cells[0].Style);
        Assert.AreEqual(StyleAttributes.Bold, style.Attributes);
        Assert.AreEqual(Underline.SingleLine, style.Underline);
        Assert.AreEqual(default, view.StyleAt(ushort.MaxValue), "an index past the table is the default style");
    }

    [TestMethod]
    public void ResizeChangesTheFrameAndKeepsTheText()
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed("hello"u8);
        terminal.Update();

        terminal.ResizeBegin();
        terminal.Resize(40, 10, 400, 200);
        Assert.IsTrue(terminal.Update());
        FrameView view = terminal.View;
        Assert.AreEqual((40, 10), (view.Cols, view.Rows));
        Assert.IsTrue(view.Full);
        Assert.AreEqual(10, view.Dirty.Length);
        Assert.AreEqual("hello", terminal.ReadText(0, 1));
    }

    [TestMethod]
    public void TheBellIsAnEvent()
    {
        using var terminal = new Terminal(20, 4);
        Assert.IsFalse(terminal.TryPollEvent(out _));

        terminal.Feed("\a\a"u8);
        Assert.IsTrue(terminal.TryPollEvent(out TerminalEvent bell));
        Assert.AreEqual(EventKind.Bell, bell.Kind);
        Assert.IsFalse(terminal.TryPollEvent(out _), "bells since the last poll are one event");
    }

    [TestMethod]
    public void AnInvalidArgumentIsAnException()
    {
        ScullException created = Assert.ThrowsExactly<ScullException>(() => new Terminal(0, 24));
        Assert.AreEqual(Status.Invalid, created.Status);

        using var terminal = new Terminal(20, 4);
        ScullException resized = Assert.ThrowsExactly<ScullException>(() => terminal.Resize(0, 3));
        Assert.AreEqual(Status.Invalid, resized.Status);
        Assert.IsFalse(terminal.IsPoisoned);
        terminal.Feed("still fine"u8);
    }

    [TestMethod]
    public void DisposingTwiceIsSafeAndLaterCallsThrow()
    {
        var terminal = new Terminal(20, 4);
        terminal.Feed("x"u8);
        terminal.Update();
        terminal.Dispose();
        terminal.Dispose();

        Assert.ThrowsExactly<ObjectDisposedException>(() => terminal.Feed("x"u8));
        Assert.ThrowsExactly<ObjectDisposedException>(() => terminal.Update());
        Assert.ThrowsExactly<ObjectDisposedException>(() => terminal.ReadText());
        Assert.ThrowsExactly<ObjectDisposedException>(() => _ = terminal.View.Cols);
    }

    [TestMethod]
    public void AHandleIsFreedOnceAndNotWhileLeased()
    {
        var handle = new NativeHandle(0x1000, &CountFree);
        using (new Lease(handle))
        {
            handle.Dispose();
            Assert.AreEqual(0, Volatile.Read(ref frees), "a call in flight keeps the handle alive");
        }
        Assert.AreEqual(1, Volatile.Read(ref frees));
        handle.Dispose();
        Assert.AreEqual(1, Volatile.Read(ref frees));
        Assert.ThrowsExactly<ObjectDisposedException>(() => new Lease(handle).Dispose());
    }

    [TestMethod]
    public void APanicPoisonsOnlyItsTerminal()
    {
        using var crashed = new Terminal(20, 4);
        using var sibling = new Terminal(20, 4);

        using (Lease lease = crashed.LeaseTerm())
        {
            Assert.AreEqual(tt_status.TT_PANIC, TestPanic(lease.Pointer));
        }
        TerminalPoisonedException poisoned = Assert.ThrowsExactly<TerminalPoisonedException>(() => crashed.Feed("x"u8));
        Assert.AreEqual(Status.Poisoned, poisoned.Status);
        Assert.IsTrue(crashed.IsPoisoned);

        sibling.Feed("alive"u8);
        Assert.IsTrue(sibling.Update());
        Assert.AreEqual("alive", sibling.ReadText(0, 1));
        Assert.IsFalse(sibling.IsPoisoned);
    }

    private static void CountFree(nint pointer)
    {
        Assert.AreEqual(0x1000, pointer);
        Interlocked.Increment(ref frees);
    }
}
