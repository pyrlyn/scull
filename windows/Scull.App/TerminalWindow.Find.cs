using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Scull.Core;

namespace Scull.App;

// Find in the scrollback, as TerminalView+Find.swift joins the bar to the state.
internal sealed partial class TerminalWindow
{
    // A step and a count rescan the whole history, so they wait for typing to pause.
    private static readonly TimeSpan FindSettle = TimeSpan.FromMilliseconds(250);

    private readonly FindBar findBar = new();
    private DispatcherQueueTimer? findTimer;

    /// <summary>The pane the bar searches; null while it is closed.</summary>
    private TerminalPane? findPane;

    private void WireFind()
    {
        findTimer = DispatcherQueue.CreateTimer();
        findTimer.IsRepeating = false;
        findTimer.Interval = FindSettle;
        findTimer.Tick += (_, _) => Find(find => find.Settle());
        findBar.Edited += text =>
        {
            Find(find => find.Edit(text));
            findTimer.Stop();
            findTimer.Start();
        };
        findBar.Stepped += forward => Find(find => find.Step(forward));
        findBar.CaseToggled += () =>
        {
            Find(find => find.ToggleCase());
            findTimer.Stop();
            findTimer.Start();
        };
        findBar.CloseRequested += () =>
        {
            CloseFind();
            host.Focus(FocusState.Programmatic);
            Redraw(force: false);
        };
    }

    /// <summary>Opens the bar on the focused pane, ending a search left open in another one.</summary>
    private void OpenFind()
    {
        if (FocusedPane() is not { } pane)
        {
            return;
        }
        if (findPane != pane)
        {
            CloseFind();
        }
        findPane = pane;
        Apply(pane, pane.Find.Open());
        findBar.Open(pane.Find);
        if (pane.Find.Unsettled)
        {
            findTimer?.Start();
        }
        Redraw(force: false);
    }

    private void Find(Func<FindState, IReadOnlyList<FindEffect>> change)
    {
        if (findPane is { } pane)
        {
            Apply(pane, change(pane.Find));
            Redraw(force: false);
        }
    }

    /// <summary>Hides the bar and ends its search; the caller redraws.</summary>
    private void CloseFind()
    {
        findTimer?.Stop();
        findBar.Hide();
        if (findPane is { } pane)
        {
            findPane = null;
            Apply(pane, pane.Find.Close());
        }
    }

    // The search follows the pane it was opened on: moving the focus away, or
    // closing that pane or its tab, ends it.
    private void FollowFind()
    {
        if (findPane is null || findPane == FocusedPane())
        {
            return;
        }
        if (!tabs.Any(t => t.Views.ContainsValue(findPane)))
        {
            // Gone with its pane: there is no search left to end.
            findPane = null;
        }
        CloseFind();
    }

    private void Apply(TerminalPane pane, IReadOnlyList<FindEffect> effects)
    {
        foreach (FindEffect effect in effects)
        {
            switch (effect)
            {
                case FindEffect.Search search:
                    pane.Send(t => t.SearchSet(search.Pattern, search.IgnoreCase));
                    break;
                case FindEffect.Move move:
                    // The core scrolls the viewport to the match; the redraw after shows it.
                    pane.Send(t => t.SearchStep(move.Forward, out _));
                    break;
                case FindEffect.Count:
                    pane.Send(t => pane.Find.Count = t.SearchCount());
                    break;
            }
        }
        findBar.Show(pane.Find);
    }
}
