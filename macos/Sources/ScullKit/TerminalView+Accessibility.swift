// The view as an NSAccessibility text area over the core's screen text, so
// VoiceOver reads the screen by character, word and line. Line N is screen
// row N; the insertion point is the cursor. The selection is read from the
// cells the frame marks, one range per row, so a block selection is exact;
// with none selected the selection is the empty range at the cursor.

import AppKit
import CScull

extension TerminalView {
    private var screen: ScreenText { session?.screenText() ?? ScreenText("") }

    private func width(row: Int) -> (Int) -> Int {
        { [session] col in Int(session?.cell(row: row, col: col)?.width ?? 1) }
    }

    /// Cells in screen coordinates, as accessibility and input methods
    /// take them.
    func screenRect(row: Int, col: Int, cols: Int, rows: Int = 1) -> NSRect {
        let rect = NSRect(x: CGFloat(col) * cellWidth, y: CGFloat(row) * cellHeight,
                          width: CGFloat(cols) * cellWidth, height: CGFloat(rows) * cellHeight)
        guard let window else { return .zero }
        return window.convertToScreen(convert(rect, to: nil))
    }

    /// Tells VoiceOver the screen changed; skipped without it, since
    /// clients re-read the whole text on each one.
    func screenChanged() {
        guard NSWorkspace.shared.isVoiceOverEnabled else { return }
        NSAccessibility.post(element: self, notification: .valueChanged)
        // Output that rewrites the selected rows drops the selection.
        postSelectionChange()
    }

    /// Tells VoiceOver the selection changed, when it did: the ranges differ from those last posted.
    func postSelectionChange() {
        guard NSWorkspace.shared.isVoiceOverEnabled else { return }
        let now = selectedRanges
        guard now != postedSelection else { return }
        postedSelection = now
        NSAccessibility.post(element: self, notification: .selectedTextChanged)
    }

    /// The selection in the value's coordinates, a range per row; empty when nothing is selected.
    private var selectedRanges: [NSRange] { screen.ranges(of: session?.selectedSpans() ?? [], width: width(row:)) }

    private var cursorRange: NSRange {
        guard let cursor = session?.view.cursor else { return NSRange(location: 0, length: 0) }
        let row = Int(cursor.row)
        return NSRange(location: screen.index(line: row, col: Int(cursor.col), width: width(row: row)), length: 0)
    }

    public override func isAccessibilityElement() -> Bool { true }

    public override func accessibilityRole() -> NSAccessibility.Role? { .textArea }

    public override func accessibilityLabel() -> String? { "Terminal" }

    public override func accessibilityValue() -> Any? { screen.string }

    public override func accessibilityNumberOfCharacters() -> Int { screen.length }

    public override func accessibilityVisibleCharacterRange() -> NSRange {
        NSRange(location: 0, length: screen.length)
    }

    public override func accessibilitySelectedText() -> String? {
        let text = screen
        return selectedRanges.compactMap(text.substring).joined(separator: "\n")
    }

    public override func accessibilitySelectedTextRange() -> NSRange {
        let ranges = selectedRanges
        guard let first = ranges.first, let last = ranges.last else { return cursorRange }
        return NSRange(location: first.location, length: NSMaxRange(last) - first.location)
    }

    public override func accessibilitySelectedTextRanges() -> [NSValue]? {
        let ranges = selectedRanges
        return (ranges.isEmpty ? [cursorRange] : ranges).map { NSValue(range: $0) }
    }

    /// VoiceOver selecting text: the cells from the range's first character to its last. An empty
    /// range, which would only move the insertion point, clears the selection instead.
    public override func setAccessibilitySelectedTextRange(_ range: NSRange) {
        guard let session else { return }
        if let cells = screen.selection(for: range, width: width(row:)) {
            _ = session.select(TT_SELECT_CELL, row: cells.start.row, col: cells.start.col)
            _ = session.extendSelection(row: cells.end.row, col: cells.end.col)
        } else {
            _ = session.clearSelection()
        }
        if session.update() { needsDisplay = true }
        postSelectionChange()
    }

    public override func accessibilityInsertionPointLineNumber() -> Int {
        min(Int(session?.view.cursor.row ?? 0), screen.lineCount - 1)
    }

    public override func accessibilityLine(for index: Int) -> Int { screen.line(for: index) }

    public override func accessibilityRange(forLine line: Int) -> NSRange {
        screen.range(forLine: line) ?? NSRange(location: NSNotFound, length: 0)
    }

    public override func accessibilityString(for range: NSRange) -> String? { screen.substring(range) }

    public override func accessibilityAttributedString(for range: NSRange) -> NSAttributedString? {
        screen.substring(range).map { NSAttributedString(string: $0, attributes: [.font: font]) }
    }

    /// One line gives the columns the range covers; more give whole rows.
    public override func accessibilityFrame(for range: NSRange) -> NSRect {
        let text = screen
        guard range.location != NSNotFound, range.location <= text.length else { return .zero }
        let first = text.line(for: range.location)
        let last = text.line(for: max(range.location, NSMaxRange(range) - 1))
        guard first == last else {
            return screenRect(row: first, col: 0, cols: Int(session?.view.cols ?? 0), rows: last - first + 1)
        }
        let stops = text.columns(line: first, width: width(row: first))
        let start = stops.last { $0.index <= range.location }?.col ?? 0
        let end = stops.first { $0.index >= NSMaxRange(range) }?.col ?? start
        return screenRect(row: first, col: start, cols: max(1, end - start))
    }

    /// The character under a screen point.
    public override func accessibilityRange(for point: NSPoint) -> NSRange {
        let text = screen
        guard let window, cellWidth > 0, cellHeight > 0 else { return NSRange(location: NSNotFound, length: 0) }
        let local = convert(window.convertPoint(fromScreen: point), from: nil)
        let row = min(max(0, Int(local.y / cellHeight)), text.lineCount - 1)
        let index = text.index(line: row, col: max(0, Int(local.x / cellWidth)), width: width(row: row))
        guard index < text.length else { return NSRange(location: index, length: 0) }
        return (text.string as NSString).rangeOfComposedCharacterSequence(at: index)
    }
}
