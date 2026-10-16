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
    private readonly TextChangeTracker tracker = new();

    public TerminalAutomationPeer(TerminalHost owner, Func<TextSnapshot?> snapshot)
        : base(owner) => this.snapshot = snapshot;

    protected override string GetClassNameCore() => nameof(TerminalHost);

    protected override string GetNameCore() => "Terminal";

    protected override AutomationControlType GetAutomationControlTypeCore() => AutomationControlType.Document;

    protected override object GetPatternCore(PatternInterface patternInterface) =>
        patternInterface == PatternInterface.Text ? this : base.GetPatternCore(patternInterface);

    // There is no selection to make yet, so none is supported; the caret is what a client reads as the selection.
    public SupportedTextSelection SupportedTextSelection => SupportedTextSelection.None;

    public ITextRangeProvider DocumentRange => Range(s => s.Document);

    public ITextRangeProvider[] GetSelection() => [Range(s => s.Caret)];

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

    /// <summary>Raises <c>TextChanged</c> when a client listens and the text really changed.</summary>
    internal void RaiseTextChanged()
    {
        bool listening = AutomationPeer.ListenerExists(AutomationEvents.TextPatternOnTextChanged);
        if (tracker.Observe(listening, () => Current().Text) is not null)
        {
            RaiseAutomationEvent(AutomationEvents.TextPatternOnTextChanged);
        }
    }

    private TextSnapshot Current() => snapshot() ?? Empty;

    private TerminalTextRange Range(Func<TextSnapshot, TextRange> pick)
    {
        TextSnapshot now = Current();
        return new TerminalTextRange(now, pick(now), this);
    }
}
