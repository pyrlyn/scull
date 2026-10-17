using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Scull.Core;
using Windows.Foundation;

namespace Scull.App;

/// <summary>
/// The terminal in the automation tree: a group whose children are the panes of the selected tab,
/// one <see cref="TerminalPanePeer"/> each, in reading order. Which panes there are, what they
/// are called and where they sit comes from <see cref="PaneElements"/>; a pane keeps its peer
/// through splits and closes of its neighbours, so a client's reference to it stays valid.
/// </summary>
internal sealed partial class TerminalAutomationPeer : FrameworkElementAutomationPeer
{
    private readonly TerminalHost owner;
    private readonly Func<IReadOnlyList<PaneElement>> panes;
    private readonly Func<int, TextSnapshot?> snapshot;
    private readonly Action<int, CellSpan?> select;
    private readonly Action<int> focusPane;
    private readonly PaneFocusTracker focus = new();
    private readonly Dictionary<int, TerminalPanePeer> peers = [];
    private int[] ids = [];

    public TerminalAutomationPeer(
        TerminalHost owner,
        Func<IReadOnlyList<PaneElement>> panes,
        Func<int, TextSnapshot?> snapshot,
        Action<int, CellSpan?> select,
        Action<int> focusPane)
        : base(owner) => (this.owner, this.panes, this.snapshot, this.select, this.focusPane) = (owner, panes, snapshot, select, focusPane);

    protected override string GetClassNameCore() => nameof(TerminalHost);

    protected override string GetNameCore() => "Terminal";

    protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Group;

    // The panes are the children; the XAML inside the host (a swap chain and notices) is not content.
    protected override IList<AutomationPeer> GetChildrenCore() => [.. Sync()];

    /// <summary>The host's top left in screen pixels, which is where the swap chain's pixel (0, 0) is.</summary>
    internal Point ScreenOrigin
    {
        get
        {
            Rect bounds = GetBoundingRectangle();
            return new Point(bounds.X, bounds.Y);
        }
    }

    internal bool HostHasFocus => owner.FocusState != FocusState.Unfocused;

    internal void FocusPane(int pane) => focusPane(pane);

    /// <summary>
    /// A redraw happened: the panes may have changed their text, their selection, their number or
    /// place. Raises what a client listens for.
    /// </summary>
    internal void Refresh()
    {
        List<TerminalPanePeer> now = Sync();
        foreach (TerminalPanePeer pane in now)
        {
            pane.RaiseTextChanged();
        }
        AnnounceFocus(now);
    }

    // The window's keyboard focus came or went, or the focused pane moved: every such change redraws.
    private void AnnounceFocus(List<TerminalPanePeer> now)
    {
        int? focused = now.FirstOrDefault(p => p.Element.Focused)?.Element.Pane;
        if (focus.Observe(HostHasFocus, focused) is { } pane && ListenerExists(AutomationEvents.AutomationFocusChanged))
        {
            peers[pane].RaiseAutomationEvent(AutomationEvents.AutomationFocusChanged);
        }
    }

    // Brings the child peers in line with the tab's panes: a pane keeps its peer, a new one gets one.
    private List<TerminalPanePeer> Sync()
    {
        IReadOnlyList<PaneElement> elements = panes();
        HashSet<int> present = [.. elements.Select(e => e.Pane)];
        foreach (int gone in peers.Keys.Where(p => !present.Contains(p)).ToArray())
        {
            peers.Remove(gone);
        }
        List<TerminalPanePeer> ordered = [];
        foreach (PaneElement element in elements)
        {
            if (!peers.TryGetValue(element.Pane, out TerminalPanePeer? peer))
            {
                peers[element.Pane] = peer = new TerminalPanePeer(this, element, snapshot, select);
            }
            peer.Element = element;
            ordered.Add(peer);
        }
        int[] now = [.. elements.Select(e => e.Pane)];
        if (!now.SequenceEqual(ids))
        {
            ids = now;
            if (ListenerExists(AutomationEvents.StructureChanged))
            {
                RaiseAutomationEvent(AutomationEvents.StructureChanged);
            }
        }
        return ordered;
    }
}
