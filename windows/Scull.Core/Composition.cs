namespace Scull.Core;

/// <summary>
/// How many compositions an input method has open and whether the text of one that just
/// ended still has to be read. The method reports a start and an end for each; the text is
/// read in an edit session it allows only after the end, so a key that arrives in between
/// must find the session run first, or Enter would go out before the text it committed.
/// </summary>
public sealed class CompositionTracker
{
    private int open;

    public bool Active => open > 0;

    /// <summary>Exactly one composition is open: the case in which an edit changes the text on show.</summary>
    public bool OnlyOne => open == 1;

    /// <summary>A composition ended and its text has not been read.</summary>
    public bool Pending { get; private set; }

    public void Started() => open++;

    /// <summary>True when it was the last one: the text is ready to read, which is now <see cref="Pending"/>.</summary>
    public bool Ended()
    {
        if (open == 0)
        {
            return false;
        }
        open--;
        Pending |= open == 0;
        return open == 0;
    }

    /// <summary>The text was read; also what a lost focus does, since the method drops the composition.</summary>
    public void Read() => Pending = false;

    public void Reset() => (open, Pending) = (0, false);
}

/// <summary>What the input method's document holds, split for the terminal.</summary>
/// <param name="Finished">Text the terminal sends to the child now.</param>
/// <param name="Active">The composing text, shown at the cursor and not sent.</param>
/// <param name="Caret">A UTF-16 index into <paramref name="Active"/>.</param>
public readonly record struct CompositionSplit(string Finished, string Active, int Caret)
{
    /// <summary>
    /// The document is a list of runs, each composing or not. Finished text is only what
    /// comes before the first composing run: it can no longer change, while a finished-looking
    /// run between two compositions can, so that and everything after it is shown, not sent.
    /// <paramref name="caret"/> counts UTF-16 units from the start of the document.
    /// </summary>
    public static CompositionSplit Of(IEnumerable<(string Text, bool Composing)> runs, int caret)
    {
        ArgumentNullException.ThrowIfNull(runs);
        var finished = new System.Text.StringBuilder();
        var active = new System.Text.StringBuilder();
        bool composing = false;
        foreach ((string text, bool isComposing) in runs)
        {
            composing |= isComposing;
            (composing ? active : finished).Append(text);
        }
        return new CompositionSplit(finished.ToString(), active.ToString(), Math.Clamp(caret - finished.Length, 0, active.Length));
    }

    /// <summary>
    /// The clause kind of a TSF display attribute (<c>TF_DISPLAYATTRIBUTE.bAttr</c>, <c>TF_ATTR_*</c>): the
    /// clause being converted is the target, converted text is converted, and typed text, an input error
    /// and anything unknown are still input.
    /// </summary>
    public static PreeditKind KindOf(int attribute) => attribute switch
    {
        1 or 3 => PreeditKind.Target, // TF_ATTR_TARGET_CONVERTED, TF_ATTR_TARGET_NOTCONVERTED
        2 or 5 => PreeditKind.Converted, // TF_ATTR_CONVERTED, TF_ATTR_FIXEDCONVERTED
        _ => PreeditKind.Input,
    };

    /// <summary>
    /// The clauses of <see cref="Active"/> from ranges the input method marked in the document: (start, end,
    /// <c>TF_ATTR_*</c>) in characters from its start. Finished text before the composition is not in
    /// <see cref="Active"/>, so offsets shift past it, and what falls outside the active text or is empty is dropped.
    /// </summary>
    public IReadOnlyList<PreeditClause> Clauses(IEnumerable<(int Start, int End, int Attribute)> marked)
    {
        ArgumentNullException.ThrowIfNull(marked);
        var clauses = new List<PreeditClause>();
        foreach ((int start, int end, int attribute) in marked.OrderBy(m => m.Start))
        {
            int from = Math.Clamp(start - Finished.Length, 0, Active.Length), to = Math.Clamp(end - Finished.Length, 0, Active.Length);
            if (from < to)
            {
                clauses.Add(new PreeditClause(from, to, KindOf(attribute)));
            }
        }
        return clauses;
    }
}
