import AppKit
import CScull
import Testing
@testable import ScullKit

// "ab cd\n中x y\n\n": 中 is wide (line 1, columns 0-1), so its tail cell has width 0.
private func makeText() -> ScreenText { ScreenText("ab cd\n中x y\n\n") }
private func widths(_ line: Int) -> (Int) -> Int { { col in line == 1 ? [0: 2, 1: 0][col] ?? 1 : 1 } }

private func ranges(_ spans: [SelectedSpan]) -> [NSRange] { makeText().ranges(of: spans, width: widths) }

private func cells(_ location: Int, _ length: Int) -> ScreenSelection? {
    makeText().selection(for: NSRange(location: location, length: length), width: widths)
}

@Test func spansMapToTheRangesOfTheCharactersTheyCover() {
    #expect(ranges([SelectedSpan(line: 0, col: 1, endCol: 4)]) == [NSRange(location: 1, length: 3)])
    // The tail of a wide character belongs to its head, at either end.
    #expect(ranges([SelectedSpan(line: 1, col: 1, endCol: 3)]) == [NSRange(location: 6, length: 2)])
    #expect(ranges([SelectedSpan(line: 1, col: 0, endCol: 1)]) == [NSRange(location: 6, length: 1)])
    #expect(ranges([SelectedSpan(line: 0, col: 3, endCol: 5), SelectedSpan(line: 1, col: 2, endCol: 3)])
        == [NSRange(location: 3, length: 2), NSRange(location: 7, length: 1)])
}

@Test func cellsPastTheTextOrOnBlankLinesSelectNothing() {
    #expect(ranges([SelectedSpan(line: 0, col: 7, endCol: 10)]).isEmpty)
    #expect(ranges([SelectedSpan(line: 2, col: 0, endCol: 10)]).isEmpty)
    #expect(ranges([SelectedSpan(line: 9, col: 0, endCol: 10)]).isEmpty, "a line that is not there")
    #expect(ranges([SelectedSpan(line: 0, col: 3, endCol: 10)]) == [NSRange(location: 3, length: 2)],
            "a selection running into the blanks stops at the text")
}

@Test func rangesMapBackToTheirFirstAndLastCells() {
    #expect(cells(1, 3) == ScreenSelection(start: (0, 1), end: (0, 3)))
    #expect(cells(6, 2) == ScreenSelection(start: (1, 0), end: (1, 2)), "a wide character covers two columns")
    #expect(cells(4, 3) == ScreenSelection(start: (0, 4), end: (1, 1)), "across a newline")
    #expect(cells(4, 2) == ScreenSelection(start: (0, 4), end: (0, 4)), "a trailing newline is not a cell")
    #expect(cells(5, 1) == nil, "a newline alone selects nothing")
    #expect(cells(99, 1) == nil && cells(NSNotFound, 0) == nil)
    #expect(cells(10, 2) == nil, "blank lines hold no character")
}

@Test func aRangeInsideASurrogatePairStillFindsItsCell() {
    let emoji = ScreenText("😀a")
    let wide: (Int) -> (Int) -> Int = { _ in { [0: 2, 1: 0][$0] ?? 1 } }
    #expect(emoji.selection(for: NSRange(location: 1, length: 1), width: wide) == ScreenSelection(start: (0, 0), end: (0, 1)))
    #expect(emoji.ranges(of: [SelectedSpan(line: 0, col: 1, endCol: 2)], width: wide) == [NSRange(location: 0, length: 2)])
}

@MainActor
private func screen(_ feed: String) throws -> (TerminalView, NSWindow) {
    let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 100))
    let session = try TerminalSession(cols: 10, rows: 3)
    #expect(session.feed(Array(feed.utf8)) == TT_OK)
    session.update()
    view.session = session
    view.setFrameSize(NSSize(width: 10.5 * view.cellWidth, height: 3.5 * view.cellHeight))
    let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView = view
    return (view, window)
}

@MainActor
@Test func theSelectionIsTheMarkedCellsInTheValuesCoordinates() throws {
    let (view, _) = try screen("hello\r\n中ab")
    let session = try #require(view.session)
    let cursor = view.accessibilitySelectedTextRange()
    #expect(cursor.length == 0)

    #expect(session.select(TT_SELECT_WORD, row: 0, col: 1) == TT_OK)
    session.update()
    #expect(view.accessibilitySelectedText() == "hello")
    #expect(view.accessibilitySelectedTextRange() == NSRange(location: 0, length: 5))
    #expect(view.accessibilitySelectedTextRanges()?.map(\.rangeValue) == [NSRange(location: 0, length: 5)])

    #expect(session.clearSelection() == TT_OK)
    session.update()
    #expect(view.accessibilitySelectedText() == "")
    #expect(view.accessibilitySelectedTextRange() == cursor, "no selection falls back to the insertion point")
}

@MainActor
@Test func aBlockIsOneRangePerRowAndWrappedRowsStaySeparateLines() throws {
    let (view, _) = try screen("hello\r\n中ab")
    let session = try #require(view.session)
    #expect(session.select(TT_SELECT_BLOCK, row: 0, col: 0) == TT_OK)
    #expect(session.extendSelection(row: 1, col: 1) == TT_OK)
    session.update()
    #expect(view.accessibilitySelectedTextRanges()?.map(\.rangeValue) == [NSRange(location: 0, length: 2), NSRange(location: 6, length: 1)])
    #expect(view.accessibilitySelectedText() == "he\n中")
    #expect(view.accessibilitySelectedTextRange() == NSRange(location: 0, length: 7), "the hull")

    // Ten columns wrap "hello worldfoo": the value has a line per row, so the text does too.
    let (wrapped, _) = try screen("hello worldfoo")
    let wrap = try #require(wrapped.session)
    #expect(wrap.select(TT_SELECT_CELL, row: 0, col: 6) == TT_OK)
    #expect(wrap.extendSelection(row: 1, col: 1) == TT_OK)
    wrap.update()
    #expect(wrapped.accessibilitySelectedText() == "worl\ndf")
    #expect(wrap.selectionText() == "worldf", "the copy joins the wrapped rows")
}

@MainActor
@Test func voiceOverSelectingARangeSelectsItsCells() throws {
    let (view, _) = try screen("hello\r\n中ab")
    let session = try #require(view.session)
    view.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 3))
    #expect(session.selectionText() == "ell")
    #expect(view.accessibilitySelectedText() == "ell")

    view.setAccessibilitySelectedTextRange(NSRange(location: 6, length: 2))
    #expect(session.selectionText() == "中a")

    view.setAccessibilitySelectedTextRange(NSRange(location: 3, length: 0))
    #expect(session.hasSelection == false, "an empty range clears")

    view.setAccessibilitySelectedTextRange(NSRange(location: 1, length: 3))
    view.setAccessibilitySelectedTextRange(NSRange(location: 5, length: 1))
    #expect(session.hasSelection == false, "a newline alone selects nothing")
}
