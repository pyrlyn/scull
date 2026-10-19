using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Scull.Core;

namespace Scull.App;

/// <summary>
/// The control that takes the window's keyboard focus and so stands for the terminal in the
/// automation tree. The panes are regions of one swap chain, not elements of their own, so
/// its peer lists them as children: <paramref name="panes"/> describes those of the selected
/// tab, and the text, the selection and the focus request of a pane come back by its id.
/// </summary>
internal sealed partial class TerminalHost(
    Func<IReadOnlyList<PaneElement>> panes,
    Func<int, TextSnapshot?> snapshot,
    Action<int, CellSpan?> select,
    Action<int> focusPane) : UserControl
{
    private TerminalAutomationPeer? peer;

    protected override AutomationPeer OnCreateAutomationPeer() => peer ??= new TerminalAutomationPeer(this, panes, snapshot, select, focusPane);

    /// <summary>
    /// The text, the selection or the panes may have changed. Does nothing until a client has asked
    /// for the peer, and the peer reads a pane only while a client listens for its text.
    /// </summary>
    public void TextChanged() => peer?.Refresh();
}
