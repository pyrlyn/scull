namespace Scull.Core;

/// <summary>A core call the find bar's state asks for.</summary>
public abstract record FindEffect
{
    private FindEffect() { }

    /// <summary>Replace the search; an empty pattern ends it.</summary>
    public sealed record Search(string Pattern, bool IgnoreCase) : FindEffect;

    /// <summary>Step to the next match (forward) or the previous one.</summary>
    public sealed record Move(bool Forward) : FindEffect;

    /// <summary>Count the matches.</summary>
    public sealed record Count : FindEffect;
}

/// <summary>
/// Find in the scrollback, as FindBar.swift's <c>FindState</c> keeps it: what
/// the bar shows and which core calls each change asks for. The window runs the effects.
/// </summary>
public sealed class FindState
{
    private static readonly FindEffect[] None = [];

    public bool IsOpen { get; private set; }

    /// <summary>Kept across a close, so the bar reopens on the last search.</summary>
    public string Pattern { get; private set; } = "";

    public bool IgnoreCase { get; private set; } = true;

    /// <summary>Counted once the pattern settles; null until then.</summary>
    public int? Count { get; set; }

    /// <summary>A step and a count both rescan the whole history, so typing waits for the pattern to settle before either runs.</summary>
    public bool Unsettled { get; private set; }

    public bool CanStep => IsOpen && Pattern.Length > 0;

    public IReadOnlyList<FindEffect> Open()
    {
        if (IsOpen)
        {
            return None;
        }
        IsOpen = true;
        return Pattern.Length == 0 ? None : Edit(Pattern);
    }

    public IReadOnlyList<FindEffect> Edit(string text)
    {
        ArgumentNullException.ThrowIfNull(text);
        if (!IsOpen)
        {
            return None;
        }
        (Pattern, Count, Unsettled) = (text, null, text.Length > 0);
        return [new FindEffect.Search(text, IgnoreCase)];
    }

    public IReadOnlyList<FindEffect> ToggleCase()
    {
        IgnoreCase = !IgnoreCase;
        return Edit(Pattern);
    }

    /// <summary>The pattern stopped changing: show the first match and count.</summary>
    public IReadOnlyList<FindEffect> Settle()
    {
        if (!Unsettled || !CanStep)
        {
            return None;
        }
        Unsettled = false;
        return [new FindEffect.Move(Forward: true), new FindEffect.Count()];
    }

    public IReadOnlyList<FindEffect> Step(bool forward)
    {
        if (!CanStep)
        {
            return None;
        }
        // Stepping now stands in for the settle's own first step.
        Unsettled = false;
        return Count is null ? [new FindEffect.Move(forward), new FindEffect.Count()] : [new FindEffect.Move(forward)];
    }

    public IReadOnlyList<FindEffect> Close()
    {
        if (!IsOpen)
        {
            return None;
        }
        (IsOpen, Count, Unsettled) = (false, null, false);
        return [new FindEffect.Search("", IgnoreCase)];
    }

    /// <summary>The count as the bar shows it; empty until counted.</summary>
    public string Label => Count switch
    {
        null => "",
        0 => "No matches",
        1 => "1 match",
        int n when n >= Terminal.MaxSearchMatches => $"{n}+ matches",
        int n => $"{n} matches",
    };
}
