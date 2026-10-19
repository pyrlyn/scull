namespace Scull.Core.Tests;

[TestClass]
public sealed class SelectionGestureTests
{
    private const KeyModifiers Shift = KeyModifiers.Shift;

    [TestMethod]
    [DataRow(1, KeyModifiers.None, false, false, SelectionKind.Cell)]
    [DataRow(2, KeyModifiers.None, false, false, SelectionKind.Word)]
    [DataRow(3, KeyModifiers.None, false, false, SelectionKind.Line)]
    [DataRow(7, KeyModifiers.None, false, false, SelectionKind.Line)]
    [DataRow(1, KeyModifiers.Alt, false, false, SelectionKind.Block)]
    [DataRow(1, KeyModifiers.Alt | KeyModifiers.Control, false, false, SelectionKind.Cell)] // AltGr
    [DataRow(2, KeyModifiers.Shift, false, false, SelectionKind.Word)]
    [DataRow(1, KeyModifiers.Shift, false, false, SelectionKind.Cell)]
    [DataRow(1, KeyModifiers.Shift, true, true, SelectionKind.Cell)]
    [DataRow(2, KeyModifiers.Shift, true, true, SelectionKind.Cell)]
    [DataRow(2, KeyModifiers.None, true, false, SelectionKind.Word)]
    public void APressStartsOrExtends(int clicks, KeyModifiers held, bool hasSelection, bool extend, SelectionKind kind) =>
        Assert.AreEqual(new SelectionPress(extend, kind), SelectionGesture.Press(clicks, held, hasSelection));

    [TestMethod]
    public void OnlyAPlainCellOrBlockPressClearsOnRelease()
    {
        Assert.IsTrue(SelectionGesture.Press(1, KeyModifiers.None, false).ClearsOnRelease);
        Assert.IsTrue(SelectionGesture.Press(1, KeyModifiers.Alt, false).ClearsOnRelease);
        Assert.IsFalse(SelectionGesture.Press(2, KeyModifiers.None, false).ClearsOnRelease);
        Assert.IsFalse(SelectionGesture.Press(3, KeyModifiers.None, false).ClearsOnRelease);
        Assert.IsFalse(SelectionGesture.Press(1, Shift, true).ClearsOnRelease, "extending keeps what it grew");
    }

    [TestMethod]
    public void OnlyShiftForcesTheHostsSelection()
    {
        Assert.IsTrue(SelectionGesture.ForcesSelection(Shift | KeyModifiers.Control));
        Assert.IsFalse(SelectionGesture.ForcesSelection(KeyModifiers.Control | KeyModifiers.Alt));
        Assert.IsFalse(SelectionGesture.ForcesSelection(KeyModifiers.None));
    }

    [TestMethod]
    public void PressesInTheSameSpotWithinTheIntervalCountUp()
    {
        var clicks = new ClickCounter();

        Assert.AreEqual(1, clicks.Press(1000, 50, 50, 500, 2));
        Assert.AreEqual(2, clicks.Press(1400, 51, 49, 500, 2));
        Assert.AreEqual(3, clicks.Press(1800, 50, 50, 500, 2), "the gap counts from the last click");
        Assert.AreEqual(4, clicks.Press(2200, 50, 50, 500, 2));
    }

    [TestMethod]
    [DataRow(2001L, 50.0, 50.0)] // too late
    [DataRow(1400L, 53.0, 50.0)] // too far across
    [DataRow(1400L, 50.0, 47.0)] // too far down
    [DataRow(900L, 50.0, 50.0)] // a clock that ran backwards
    public void ALatePressOrAFarOneStartsANewSeries(long ms, double x, double y)
    {
        var clicks = new ClickCounter();
        clicks.Press(1000, 50, 50, 1000, 2);

        Assert.AreEqual(1, clicks.Press(ms, x, y, ms == 2001 ? 1000 : 500, 2));
    }

    [TestMethod]
    public void ADragExtendsOnlyWhenTheCellChangesOrTheViewScrolls()
    {
        var drag = new SelectionDrag(3, 1, clearsOnRelease: true);

        Assert.IsFalse(drag.Move(3, 1, scrolled: false));
        Assert.IsTrue(drag.ClearsOnRelease, "a wobble inside the cell is still a click");
        Assert.IsTrue(drag.Move(4, 1, scrolled: false));
        Assert.IsFalse(drag.ClearsOnRelease);
        Assert.IsFalse(drag.Move(4, 1, scrolled: false));
        Assert.IsTrue(drag.Move(4, 1, scrolled: true), "the selection grows with the history");
    }

    [TestMethod]
    [DataRow(-0.5, 1)]
    [DataRow(-300.0, 1)]
    [DataRow(0.0, 0)]
    [DataRow(599.9, 0)]
    [DataRow(600.0, -1)]
    [DataRow(900.0, -1)]
    public void PastAnEdgeTheHistoryScrollsARowPerMotion(double y, int rows) =>
        Assert.AreEqual(rows, SelectionDrag.EdgeScroll(y, 600));
}
