using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation.Provider;
using Scull.Core;
using Uia = Microsoft.UI.Xaml.Automation.Text;

namespace Scull.App;

/// <summary>
/// A range of the terminal's text for UI Automation. It holds the snapshot it was made from,
/// so what a client reads stays consistent while output goes on; the client asks again for a
/// fresh range to see the new text. The unit and endpoint enums of UI Automation have the
/// same values as <see cref="TextUnit"/> and <see cref="TextEndpoint"/>, so they are cast.
/// </summary>
internal sealed partial class TerminalTextRange(TextSnapshot snapshot, TextRange range, TerminalPanePeer owner) : ITextRangeProvider
{
    private TextRange range = range;

    public ITextRangeProvider Clone() => new TerminalTextRange(snapshot, range, owner);

    public bool Compare(ITextRangeProvider textRangeProvider) => Other(textRangeProvider).range == range;

    public int CompareEndpoints(Uia.TextPatternRangeEndpoint endpoint, ITextRangeProvider targetRange, Uia.TextPatternRangeEndpoint targetEndpoint) =>
        range.CompareEndpoints(Endpoint(endpoint), Other(targetRange).range, Endpoint(targetEndpoint));

    public void ExpandToEnclosingUnit(Uia.TextUnit unit) =>
        range = snapshot.Text.ExpandToEnclosingUnit(range, Unit(unit));

    // Colours, fonts and the like are not text attributes a terminal reports.
    public ITextRangeProvider FindAttribute(int attributeId, object value, bool backward) => null!;

    public ITextRangeProvider FindText(string text, bool backward, bool ignoreCase) =>
        snapshot.Text.Find(range, text, backward, ignoreCase) is { } found ? new TerminalTextRange(snapshot, found, owner) : null!;

    public object GetAttributeValue(int attributeId) => DependencyProperty.UnsetValue;

    /// <summary>Left, top, width and height of each row's rectangle in turn, in screen pixels.</summary>
    public void GetBoundingRectangles(out double[] returnValue)
    {
        Windows.Foundation.Point origin = owner.ScreenOrigin;
        returnValue =
        [
            .. snapshot.Rectangles(range).SelectMany(r => new[] { origin.X + r.X, origin.Y + r.Y, r.Width, r.Height }),
        ];
    }

    public IRawElementProviderSimple GetEnclosingElement() => owner.Provider;

    public string GetText(int maxLength) => snapshot.Text.GetText(range, maxLength);

    public int Move(Uia.TextUnit unit, int count)
    {
        range = snapshot.Text.Move(range, Unit(unit), count, out int moved);
        return moved;
    }

    public void MoveEndpointByRange(Uia.TextPatternRangeEndpoint endpoint, ITextRangeProvider targetRange, Uia.TextPatternRangeEndpoint targetEndpoint) =>
        range = range.WithEndpoint(Endpoint(endpoint), Other(targetRange).range.At(Endpoint(targetEndpoint)));

    public int MoveEndpointByUnit(Uia.TextPatternRangeEndpoint endpoint, Uia.TextUnit unit, int count)
    {
        range = snapshot.Text.MoveEndpointByUnit(range, Endpoint(endpoint), Unit(unit), count, out int moved);
        return moved;
    }

    // An empty range clears the selection: the terminal's caret is its cursor, which a client cannot move.
    public void Select() => owner.Select(snapshot.CellsOf(range));

    // The core has one selection, so there is nothing to add to or remove from.
    public void AddToSelection() => throw new NotSupportedException("the terminal has one selection");

    public void RemoveFromSelection() => throw new NotSupportedException("the terminal has one selection");

    // The terminal scrolls by wheel, not by a client's request.
    public void ScrollIntoView(bool alignToTop)
    {
    }

    public IRawElementProviderSimple[] GetChildren() => [];

    private static TerminalTextRange Other(ITextRangeProvider provider) =>
        provider as TerminalTextRange ?? throw new ArgumentException("a range of another provider", nameof(provider));

    private static TextUnit Unit(Uia.TextUnit unit) => (TextUnit)(int)unit;

    private static TextEndpoint Endpoint(Uia.TextPatternRangeEndpoint endpoint) => (TextEndpoint)(int)endpoint;
}
