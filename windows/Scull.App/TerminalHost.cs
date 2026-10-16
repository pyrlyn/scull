using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Scull.Core;

namespace Scull.App;

/// <summary>
/// The control that takes the window's keyboard focus and so stands for the terminal in the
/// automation tree. The panes are regions of one swap chain, not elements of their own, so
/// this one exposes the text of the focused pane of the selected tab: where the typing goes.
/// </summary>
internal sealed partial class TerminalHost(Func<TextSnapshot?> snapshot) : UserControl
{
    private TerminalAutomationPeer? peer;

    protected override AutomationPeer OnCreateAutomationPeer() => peer ??= new TerminalAutomationPeer(this, snapshot);

    /// <summary>
    /// The text may have changed. Does nothing until a client has asked for the peer, and the
    /// peer reads the text only while a client listens for text changes.
    /// </summary>
    public void TextChanged() => peer?.RaiseTextChanged();
}
