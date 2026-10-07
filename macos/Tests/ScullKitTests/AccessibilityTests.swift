import AppKit
import CScull
import Testing
@testable import ScullKit

private func narrow(_: Int) -> Int { 1 }

@Test func screenTextMapsOffsetsToLines() {
    let text = ScreenText("ab\n\n😀c")
    #expect(text.length == 7 && text.lineCount == 3)
    #expect([0, 2, 3, 4, 6, 7, 99, -1].map(text.line(for:)) == [0, 0, 1, 2, 2, 2, 2, 0])
    #expect(text.range(forLine: 0) == NSRange(location: 0, length: 3))
    #expect(text.range(forLine: 1) == NSRange(location: 3, length: 1))
    #expect(text.range(forLine: 2) == NSRange(location: 4, length: 3))
    #expect(text.range(forLine: 3) == nil && text.range(forLine: -1) == nil)
    #expect(text.substring(NSRange(location: 4, length: 3)) == "😀c")
    #expect(text.substring(NSRange(location: 5, length: 9)) == nil)
    #expect(text.substring(NSRange(location: NSNotFound, length: 0)) == nil)
    #expect(ScreenText("").lineCount == 1 && ScreenText("").range(forLine: 0) == NSRange(location: 0, length: 0))
}

@Test func screenTextWalksColumnsOverCellWidths() {
    // 中 takes two cells; the row reports 2 at its head.
    let text = ScreenText("x\n中a😀")
    let widths: (Int) -> Int = { [0: 2, 1: 0, 2: 1, 3: 2][$0] ?? 1 }
    let stops = text.columns(line: 1, width: widths)
    #expect(stops.map(\.index) == [2, 3, 4, 6])
    #expect(stops.map(\.col) == [0, 2, 3, 5])
    #expect(text.index(line: 1, col: 1, width: widths) == 2, "the tail of a wide cell is its head")
    #expect(text.index(line: 1, col: 4, width: widths) == 4)
    #expect(text.index(line: 1, col: 40, width: widths) == 6)
    #expect(text.index(line: 0, col: 1, width: narrow) == 1, "past the text is the newline")
    #expect(text.columns(line: 7, width: narrow).isEmpty)
}

@MainActor
private func screen(_ feed: String) throws -> (TerminalView, NSWindow) {
    let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 100))
    // A terminal with no child; the text comes from `feed` only.
    let session = try TerminalSession(cols: 10, rows: 3)
    #expect(session.feed(Array(feed.utf8)) == TT_OK)
    session.update()
    view.session = session
    // Sized to the grid, so joining the window does not resize it.
    view.setFrameSize(NSSize(width: 10.5 * view.cellWidth, height: 3.5 * view.cellHeight))
    let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView = view
    return (view, window)
}

@MainActor
@Test func theViewIsATextAreaOverTheScreen() throws {
    let (view, _) = try screen("$ ls\r\n中b\r\n$ ")
    #expect(view.isAccessibilityElement() && view.accessibilityRole() == .textArea)
    #expect(view.accessibilityValue() as? String == "$ ls\n中b\n$")
    #expect(view.accessibilityNumberOfCharacters() == 9)
    #expect(view.accessibilityVisibleCharacterRange() == NSRange(location: 0, length: 9))
    #expect(view.accessibilityLine(for: 6) == 1)
    #expect(view.accessibilityRange(forLine: 1) == NSRange(location: 5, length: 3))
    #expect(view.accessibilityString(for: NSRange(location: 5, length: 2)) == "中b")
    #expect(view.accessibilityAttributedString(for: NSRange(location: 0, length: 4))?.string == "$ ls")
    // The cursor sits after "$ " on the last row, past the trimmed text.
    #expect(view.accessibilityInsertionPointLineNumber() == 2)
    #expect(view.accessibilitySelectedTextRange() == NSRange(location: 9, length: 0))
    #expect(view.accessibilitySelectedText() == "")
}

@MainActor
@Test func rangesAndScreenPointsMeetAtTheCells() throws {
    let (view, _) = try screen("$ ls\r\n中b")
    // "b" is in the third column of the second row, behind the wide 中.
    let b = view.screenRect(row: 1, col: 2, cols: 1)
    #expect(view.accessibilityFrame(for: NSRange(location: 6, length: 1)) == b)
    #expect(view.accessibilityFrame(for: NSRange(location: 5, length: 1)) == view.screenRect(row: 1, col: 0, cols: 2))
    #expect(view.accessibilityFrame(for: NSRange(location: 2, length: 4)) == view.screenRect(row: 0, col: 0, cols: 10,
                                                                                          rows: 2))
    #expect(view.accessibilityFrame(for: NSRange(location: 99, length: 1)) == .zero)
    #expect(view.accessibilityRange(for: NSPoint(x: b.midX, y: b.midY)) == NSRange(location: 6, length: 1))
    let tail = view.screenRect(row: 1, col: 1, cols: 1)
    #expect(view.accessibilityRange(for: NSPoint(x: tail.midX, y: tail.midY)) == NSRange(location: 5, length: 1))
}

@MainActor
@Test func newOutputIsReadAfterTheNextUpdate() throws {
    let (view, _) = try screen("a")
    #expect(view.accessibilityValue() as? String == "a\n\n", "a line per row, blank ones too")
    #expect(view.session?.feed(Array("b".utf8)) == TT_OK)
    #expect(view.accessibilityValue() as? String == "a\n\n", "the text matches the frame on screen")
    view.session?.update()
    #expect(view.accessibilityValue() as? String == "ab\n\n")
}
