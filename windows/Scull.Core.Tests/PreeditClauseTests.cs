using System.Runtime.InteropServices;
using Scull.Native;

namespace Scull.Core.Tests;

[TestClass]
public sealed class PreeditClauseTests
{
    private static (PreeditKind Kind, int Col, int Cols)[] Spans(Terminal terminal)
    {
        var found = new List<(PreeditKind, int, int)>();
        foreach (PreeditSpan span in terminal.View.PreeditSpans)
        {
            found.Add((span.Kind, span.Col, span.Cols));
        }
        return [.. found];
    }

    [TestMethod]
    public void ClausesInUtf16ComeBackAsRunsOfColumns()
    {
        using var terminal = new Terminal(20, 4);
        terminal.Feed("ab"u8);

        // "日本" converted, "ご" the target, "x" still raw: UTF-16 offsets 0-2, 2-3, 3-4.
        terminal.SetPreedit(
            "日本ごx",
            caret: 3,
            [new(0, 2, PreeditKind.Converted), new(2, 3, PreeditKind.Target), new(3, 4, PreeditKind.Input)]);
        Assert.IsTrue(terminal.Update());

        CollectionAssert.AreEqual(
            new[] { (PreeditKind.Converted, 2, 4), (PreeditKind.Target, 6, 2), (PreeditKind.Input, 8, 1) },
            Spans(terminal));
        Assert.AreEqual(new Preedit(0, 2, 7), terminal.View.Preedit);
    }

    [TestMethod]
    public void AClauseAfterAnAstralCharacterIsCountedInBytesNotUnits()
    {
        using var terminal = new Terminal(20, 4);

        // 😀 is two UTF-16 units and four bytes, two columns.
        terminal.SetPreedit("😀ab", caret: 4, [new(2, 3, PreeditKind.Target)]);
        terminal.Update();

        CollectionAssert.AreEqual(
            new[] { (PreeditKind.Input, 0, 2), (PreeditKind.Target, 2, 1), (PreeditKind.Input, 3, 1) },
            Spans(terminal),
            "the clause names only the a");
    }

    [TestMethod]
    public void AnOffsetInsideASurrogatePairMovesBackToItsStart()
    {
        Assert.AreEqual((nuint)1, PreeditMarks.ByteOffset("a😀", 1));
        Assert.AreEqual((nuint)1, PreeditMarks.ByteOffset("a😀", 2));
        Assert.AreEqual((nuint)5, PreeditMarks.ByteOffset("a😀", 3));
        Assert.AreEqual((nuint)5, PreeditMarks.ByteOffset("a😀", 99), "past the text is its end");
        Assert.AreEqual((nuint)0, PreeditMarks.ByteOffset("a😀", -3));
        Assert.AreEqual((nuint)3, PreeditMarks.ByteOffset("\ud83d", 1), "a lone surrogate is sent as the 3-byte replacement, as the text is");
    }

    [TestMethod]
    public void TextSetWithoutClausesHasNoSpansAndAFreshSetDropsOldOnes()
    {
        using var terminal = new Terminal(20, 4);
        terminal.SetPreedit("abc", 3, [new(0, 3, PreeditKind.Target)]);
        terminal.Update();
        Assert.HasCount(1, Spans(terminal));

        terminal.SetPreedit("abc", 3);
        terminal.Update();
        Assert.IsEmpty(Spans(terminal));
        Assert.AreEqual(3, terminal.View.Preedit.Cols);

        terminal.SetPreedit("abc", 3, []);
        terminal.Update();
        Assert.IsEmpty(Spans(terminal), "an empty list is no clauses");
    }

    [TestMethod]
    public void UntidyClausesAreCutByTheCoreAndNeverFail()
    {
        using var terminal = new Terminal(20, 4);

        terminal.SetPreedit(
            "abcd",
            4,
            [new(3, 99, PreeditKind.Target), new(-5, 2, PreeditKind.Converted), new(1, 1, PreeditKind.Target), new(4, 2, PreeditKind.Target)]);
        terminal.Update();

        CollectionAssert.AreEqual(
            new[] { (PreeditKind.Converted, 0, 2), (PreeditKind.Input, 2, 1), (PreeditKind.Target, 3, 1) },
            Spans(terminal));
    }

    [TestMethod]
    public void AChangeOfClauseAloneRepaintsTheRow()
    {
        using var terminal = new Terminal(20, 4);
        terminal.SetPreedit("ni", 2, [new(0, 2, PreeditKind.Converted)]);
        terminal.Update();

        terminal.SetPreedit("ni", 2, [new(0, 2, PreeditKind.Target)]);
        Assert.IsTrue(terminal.Update());

        Assert.IsTrue(terminal.View.Dirty.Contains((ushort)0));
        Assert.AreEqual(PreeditKind.Target, terminal.View.PreeditSpans[0].Kind);
    }

    [TestMethod]
    public void TheNativeSpanIsTheCoresLayout()
    {
        Assert.AreEqual(Marshal.SizeOf<tt_preedit_span>(), Marshal.SizeOf<PreeditSpan>());
    }

    [TestMethod]
    public void ACellIsMarkedByTheSpanItIsInOrPlainWhenThereAreNone()
    {
        using var terminal = new Terminal(20, 4);
        terminal.SetPreedit("日本ご", 3, [new(0, 2, PreeditKind.Converted), new(2, 3, PreeditKind.Target)]);
        terminal.Update();
        FrameView view = terminal.View;

        Assert.IsNull(PreeditMarks.At(view.Preedit, view.PreeditSpans, 0, 6), "right of the text");
        Assert.AreEqual(PreeditKind.Converted, PreeditMarks.At(view.Preedit, view.PreeditSpans, 0, 3));
        Assert.AreEqual(PreeditKind.Target, PreeditMarks.At(view.Preedit, view.PreeditSpans, 0, 4));
        Assert.IsNull(PreeditMarks.At(view.Preedit, view.PreeditSpans, 1, 0), "another row");

        terminal.SetPreedit("日本ご", 3);
        terminal.Update();
        view = terminal.View;
        Assert.AreEqual(PreeditKind.Converted, PreeditMarks.At(view.Preedit, view.PreeditSpans, 0, 5), "no clauses: one plain line");
        Assert.IsNull(PreeditMarks.At(new Preedit(0, 0, 0), default, 0, 0), "nothing composing");
    }

    [TestMethod]
    [DataRow(0, PreeditKind.Input)] // TF_ATTR_INPUT
    [DataRow(1, PreeditKind.Target)] // TF_ATTR_TARGET_CONVERTED
    [DataRow(2, PreeditKind.Converted)] // TF_ATTR_CONVERTED
    [DataRow(3, PreeditKind.Target)] // TF_ATTR_TARGET_NOTCONVERTED
    [DataRow(4, PreeditKind.Input)] // TF_ATTR_INPUT_ERROR
    [DataRow(5, PreeditKind.Converted)] // TF_ATTR_FIXEDCONVERTED
    [DataRow(-1, PreeditKind.Input)] // TF_ATTR_OTHER
    [DataRow(99, PreeditKind.Input)]
    public void ATsfDisplayAttributeSaysWhatKindOfClauseItMarks(int attribute, PreeditKind expected)
    {
        Assert.AreEqual(expected, CompositionSplit.KindOf(attribute));
    }

    [TestMethod]
    public void MarkedRangesBecomeClausesOfTheActiveTextPastTheFinishedPart()
    {
        // "ok" was finished before the composition "にほん" began, so the document is "okにほん".
        CompositionSplit split = CompositionSplit.Of([("ok", false), ("にほん", true)], caret: 5);
        (int, int, int)[] marked = [(4, 5, 5), (2, 4, 2), (0, 2, 5), (5, 5, 1), (4, 99, 3)];

        IReadOnlyList<PreeditClause> clauses = split.Clauses(marked);

        CollectionAssert.AreEqual(
            new[]
            {
                new PreeditClause(0, 2, PreeditKind.Converted),
                new PreeditClause(2, 3, PreeditKind.Converted),
                new PreeditClause(2, 3, PreeditKind.Target),
            },
            clauses.ToArray(),
            "sorted by start, the finished part and empty ranges dropped, the end cut to the text");
        Assert.IsEmpty(split.Clauses([]));
    }

    [TestMethod]
    public void ClausesMadeFromTsfMarksReachTheFrameAsSpans()
    {
        using var terminal = new Terminal(20, 4);
        CompositionSplit split = CompositionSplit.Of([("にほん", true)], caret: 3);

        terminal.SetPreedit(split.Active, split.Caret, split.Clauses([(0, 2, 2), (2, 3, 1)]));
        terminal.Update();

        CollectionAssert.AreEqual(
            new[] { (PreeditKind.Converted, 0, 4), (PreeditKind.Target, 4, 2) },
            Spans(terminal));
    }
}
