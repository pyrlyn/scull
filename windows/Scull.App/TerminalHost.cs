using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Scull.Core;

namespace Scull.App;

/// <summary>
/// The control that takes the window's keyboard focus and so stands for the terminal in the
/// automation tree. The panes are regions of one swap chain, not elements of their own, so
/// this one exposes the text of the focused pane of the selected tab: where the typing goes.
/// </summary>
internal sealed partial class TerminalHost(Func<TextSnapshot?> snapshot, Action<CellSpan?> select) : UserControl
{
    private TerminalAutomationPeer? peer;

    protected override AutomationPeer OnCreateAutomationPeer() => peer ??= new TerminalAutomationPeer(this, snapshot, select);

    /// <summary>
    /// The text or the selection may have changed. Does nothing until a client has asked for the peer,
    /// and the peer reads the pane only while a client listens for either.
    /// </summary>
    public void TextChanged() => peer?.RaiseTextChanged();
}
