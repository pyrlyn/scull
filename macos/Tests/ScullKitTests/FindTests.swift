import AppKit
import CScull
import Testing
@testable import ScullKit

@Test func findStateSearchesAtOnceAndStepsOnceSettled() {
    var find = FindState()
    #expect(find.edit("x").isEmpty, "a closed bar searches nothing")
    #expect(find.open().isEmpty)
    #expect(find.edit("fo") == [.search("fo", ignoreCase: true)])
    #expect(find.edit("foo") == [.search("foo", ignoreCase: true)])
    #expect(find.unsettled && find.label == "")
    #expect(find.settle() == [.step(forward: true), .count])
    #expect(find.settle().isEmpty, "settled once")
    find.count = 3
    #expect(find.label == "3 matches")
    #expect(find.step(forward: true) == [.step(forward: true)])
    #expect(find.step(forward: false) == [.step(forward: false)])
    // Toggling case is a new search, counted again.
    #expect(find.toggleCase() == [.search("foo", ignoreCase: false)])
    #expect(find.count == nil)
    // A step before the pause stands in for the settle.
    #expect(find.step(forward: false) == [.step(forward: false), .count])
    #expect(find.settle().isEmpty)
}

@Test func findStateClosesAndReopensOnTheLastPattern() {
    var find = FindState()
    _ = find.open()
    _ = find.edit("")
    #expect(!find.canStep && find.step(forward: true).isEmpty && find.settle().isEmpty)
    _ = find.edit("bar")
    #expect(find.close() == [.search("", ignoreCase: true)])
    #expect(!find.isOpen && !find.canStep && find.close().isEmpty)
    #expect(find.open() == [.search("bar", ignoreCase: true)])
    #expect(find.unsettled)
}

@Test func findLabelsCountMatches() {
    var find = FindState()
    for (count, label) in [(0, "No matches"), (1, "1 match"), (12, "12 matches"),
                           (Int(TT_MAX_SEARCH_MATCHES), "\(TT_MAX_SEARCH_MATCHES)+ matches")] {
        find.count = count
        #expect(find.label == label)
    }
}

@MainActor
@Test func theFindBarMarksMatchesAndEscapeEndsTheSearch() throws {
    let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 100))
    let session = try TerminalSession(cols: 10, rows: 3)
    _ = session.feed(Array("ab ab\r\nAB".utf8))
    view.session = session
    let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView = view
    let flags = { (row: Int, col: Int) in Int32(session.cell(row: row, col: col)?.flags ?? 0) }
    view.showFind(nil)
    let bar = try #require(view.findBar)
    #expect(window.firstResponder === bar.field.currentEditor())
    #expect(bar.superview === view && bar.fittingSize.width > 200)
    view.findEdited("ab")
    // Visible matches light up before the pause; the count waits for it.
    #expect(flags(1, 0) & TT_CELL_MATCH != 0 && view.find.count == nil)
    view.apply(view.find.settle())
    #expect(view.find.count == 3)
    #expect(flags(0, 0) & TT_CELL_CURRENT_MATCH != 0)
    view.findNext(nil)
    #expect(flags(0, 3) & TT_CELL_CURRENT_MATCH != 0 && flags(0, 0) & TT_CELL_CURRENT_MATCH == 0)
    view.findPrevious(nil)
    #expect(flags(0, 0) & TT_CELL_CURRENT_MATCH != 0)
    // Return and Escape reach the bar as the field editor's commands.
    let editor = NSTextView()
    #expect(bar.control(bar.field, textView: editor, doCommandBy: #selector(NSResponder.insertNewline(_:))))
    #expect(flags(0, 3) & TT_CELL_CURRENT_MATCH != 0)
    #expect(bar.control(bar.field, textView: editor, doCommandBy: #selector(NSResponder.cancelOperation(_:))))
    #expect(view.findBar == nil && !view.find.isOpen)
    #expect(flags(0, 0) == 0 && flags(1, 0) == 0)
    #expect(window.firstResponder === view)
}
