using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Automation.Provider;
using Scull.Core;
using Windows.Foundation;

namespace Scull.App;

/// <summary>
/// The terminal as a UI Automation text document. All the text logic lives in
/// <see cref="TextSnapshot"/> and <see cref="ScreenText"/>, which are tested on every runner;
/// this and <see cref="TerminalTextRange"/> only adapt them to the WinUI interfaces.
/// </summary>
internal sealed partial class TerminalAutomationPeer : FrameworkElementAutomationPeer, ITextProvider
{
    private static readonly TextSnapshot Empty = new(new ScreenText(""), default, 0, [], 0, 0);

    private readonly Func<TextSnapshot?> snapshot;
    private readonly Action<CellSpan?> select;
    private readonly TextChangeTracker tracker = new();
    private readonly SelectionTracker selection = new();

    public TerminalAutomationPeer(TerminalHost owner, Func<TextSnapshot?> snapshot, Action<CellSpan?> select)
        : base(owner) => (this.snapshot, this.select) = (snapshot, select);

    protected override string GetClassNameCore() => nameof(TerminalHost);

    protected override string GetNameCore() => "Terminal";

    protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Document;

    protected override object GetPatternCore(PatternInterface patternInterface) =>
        patternInterface == PatternInterface.Text ? this : base.GetPatternCore(patternInterface);

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
        Rect origin = GetBoundingRectangle();
        return new TerminalTextRange(now, now.RangeAt(screenLocation.X - origin.X, screenLocation.Y - origin.Y), this);
    }

    /// <summary>The host's top left in screen pixels, which is where the swap chain's pixel (0, 0) is.</summary>
    internal Point ScreenOrigin
    {
        get
        {
            Rect bounds = GetBoundingRectangle();
            return new Point(bounds.X, bounds.Y);
        }
    }

    internal IRawElementProviderSimple Provider => ProviderFromPeer(this);

    /// <summary>Selects the cells through the core; null clears the selection.</summary>
    internal void Select(CellSpan? cells) => select(cells);

    /// <summary>
    /// Raises <c>TextChanged</c> when a client listens and the text really changed, and
    /// <c>TextSelectionChanged</c> when one listens and the selection (or the caret) moved.
    /// </summary>
    internal void RaiseTextChanged()
    {
        // One capture serves both, and none is taken while nobody listens.
        TextSnapshot? now = null;
        TextSnapshot Now() => now ??= Current();
        if (tracker.Observe(AutomationPeer.ListenerExists(AutomationEvents.TextPatternOnTextChanged), () => Now().Text) is not null)
        {
            RaiseAutomationEvent(AutomationEvents.TextPatternOnTextChanged);
        }
        if (selection.Observe(AutomationPeer.ListenerExists(AutomationEvents.TextPatternOnTextSelectionChanged), () => Now().Selection ?? Now().Caret))
        {
            RaiseAutomationEvent(AutomationEvents.TextPatternOnTextSelectionChanged);
        }
    }

    private TextSnapshot Current() => snapshot() ?? Empty;

    private TerminalTextRange Range(Func<TextSnapshot, TextRange> pick)
    {
        TextSnapshot now = Current();
        return new TerminalTextRange(now, pick(now), this);
    }
}
