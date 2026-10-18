using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Scull.Core;
using Windows.Foundation;

namespace Scull.App;

/// <summary>
/// One pane as a UI Automation text document. A pane is a region of the window's swap chain, not
/// an element, so this peer has no control behind it: <see cref="PaneElements"/> says where it is
/// and what it is called, and the text logic lives in <see cref="TextSnapshot"/> and
/// <see cref="ScreenText"/>, which every runner tests. This and <see cref="TerminalTextRange"/>
/// only adapt them to the WinUI interfaces.
/// </summary>
internal sealed partial class TerminalPanePeer : AutomationPeer, ITextProvider
{
    private static readonly TextSnapshot Empty = new(new ScreenText(""), default, 0, [], 0, 0);

    private readonly TerminalAutomationPeer parent;
    private readonly Func<int, TextSnapshot?> snapshot;
    private readonly Action<int, CellSpan?> select;
    private readonly TextChangeTracker tracker = new();
    private readonly SelectionTracker selection = new();

    public TerminalPanePeer(TerminalAutomationPeer parent, PaneElement element, Func<int, TextSnapshot?> snapshot, Action<int, CellSpan?> select)
    {
        (this.parent, Element, this.snapshot, this.select) = (parent, element, snapshot, select);
        SetParent(parent);
    }

    public PaneElement Element { get; set; }

    protected override string GetClassNameCore() => nameof(TerminalPanePeer);

    protected override string GetNameCore() => Element.Name;

    protected override string GetAutomationIdCore() => $"pane-{Element.Pane}";

    protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Document;

    protected override object GetPatternCore(PatternInterface patternInterface) =>
        patternInterface == PatternInterface.Text ? this : null!;

    protected override Rect GetBoundingRectangleCore()
    {
        Point origin = parent.ScreenOrigin;
        PixelRect b = Element.Bounds;
        return new Rect(origin.X + b.X, origin.Y + b.Y, b.Width, b.Height);
    }

    protected override Point GetClickablePointCore()
    {
        Rect bounds = GetBoundingRectangleCore();
        return new Point(bounds.X + (bounds.Width / 2), bounds.Y + (bounds.Height / 2));
    }

    protected override IList<AutomationPeer> GetChildrenCore() => [];

    protected override bool HasKeyboardFocusCore() => Element.Focused && parent.HostHasFocus;

    protected override bool IsKeyboardFocusableCore() => true;

    protected override bool IsEnabledCore() => true;

    protected override bool IsOffscreenCore() => false;

    protected override bool IsContentElementCore() => true;

    protected override bool IsControlElementCore() => true;

    // The window's keyboard focus is the host's; the pane the typing goes to follows the tree.
    protected override void SetFocusCore() => parent.FocusPane(Element.Pane);

    // The core has one selection; with none, the caret is what a client reads as the selection.
    public SupportedTextSelection SupportedTextSelection => SupportedTextSelection.Single;

    public ITextRangeProvider DocumentRange => Range(s => s.Document);

    public ITextRangeProvider[] GetSelection() => [Range(s => s.Selection ?? s.Caret)];

    public ITextRangeProvider[] GetVisibleRanges() => [DocumentRange];

    // The terminal has no child elements.
    public ITextRangeProvider RangeFromChild(IRawElementProviderSimple childElement) => null!;

    public ITextRangeProvider RangeFromPoint(Point screenLocation)
    {
        TextSnapshot now = Current();
        Point origin = parent.ScreenOrigin;
        return new TerminalTextRange(now, now.RangeAt(screenLocation.X - origin.X, screenLocation.Y - origin.Y), this);
    }

    /// <summary>The host's top left in screen pixels, which is where the swap chain's pixel (0, 0) is.</summary>
    internal Point ScreenOrigin => parent.ScreenOrigin;

    internal IRawElementProviderSimple Provider => ProviderFromPeer(this);

    /// <summary>Selects the cells through the core; null clears the selection.</summary>
    internal void Select(CellSpan? cells) => select(Element.Pane, cells);

    /// <summary>
    /// Raises <c>TextChanged</c> when a client listens and the text really changed, and
    /// <c>TextSelectionChanged</c> when one listens and the selection (or the caret) moved.
    /// </summary>
    internal void RaiseTextChanged()
    {
        // One capture serves both, and none is taken while nobody listens.
        TextSnapshot? now = null;
        TextSnapshot Now() => now ??= Current();
        if (tracker.Observe(ListenerExists(AutomationEvents.TextPatternOnTextChanged), () => Now().Text) is not null)
        {
            RaiseAutomationEvent(AutomationEvents.TextPatternOnTextChanged);
        }
        if (selection.Observe(ListenerExists(AutomationEvents.TextPatternOnTextSelectionChanged), () => Now().Selection ?? Now().Caret))
        {
            RaiseAutomationEvent(AutomationEvents.TextPatternOnTextSelectionChanged);
        }
    }

    private TextSnapshot Current() => snapshot(Element.Pane) ?? Empty;

    private TerminalTextRange Range(Func<TextSnapshot, TextRange> pick)
    {
        TextSnapshot now = Current();
        return new TerminalTextRange(now, pick(now), this);
    }
}
