import AppKit
import CScull
import Testing
@testable import ScullKit

// Provided by scull-ffi's `test-hooks` feature, which `just macos-test`
// builds in; it is not in scull.h because no shipped library has it.
@_silgen_name("tt_term_test_panic")
private func tt_term_test_panic(_ term: OpaquePointer?) -> tt_status

@Test func splittingFocusesTheNewPaneAfterTheOldOne() {
    var tree = PaneTree()
    #expect(tree.panes == [0] && tree.focused == 0)
    let right = tree.splitFocused(.sideBySide)
    #expect(tree.panes == [0, right] && tree.focused == right)
    // Splitting the first pane again nests inside its share only.
    tree.focus(0)
    let below = tree.splitFocused(.stacked)
    #expect(tree.panes == [0, below, right])
    #expect(tree.root == .split(.sideBySide, .split(.stacked, .leaf(0), .leaf(below)), .leaf(right)))
}

@Test func closingGivesTheShareToTheSiblingAndMovesFocus() {
    var tree = PaneTree()
    let b = tree.splitFocused(.sideBySide)
    let c = tree.splitFocused(.stacked)
    var closed = tree.close(c)
    #expect(closed)
    #expect(tree.root == .split(.sideBySide, .leaf(0), .leaf(b)))
    #expect(tree.focused == b, "the pane before takes it when the last one closes")
    tree.focus(0)
    closed = tree.close(0)
    #expect(closed)
    #expect(tree.root == .leaf(b) && tree.focused == b)
    closed = tree.close(b)
    #expect(!closed, "the last pane is the window's to close")
    closed = tree.close(99)
    #expect(!closed)
}

@Test func focusWalksTheReadingOrderAndWraps() {
    var tree = PaneTree()
    let b = tree.splitFocused(.sideBySide)
    let c = tree.splitFocused(.stacked)
    tree.focusNext()
    #expect(tree.focused == 0)
    tree.focusNext(by: -1)
    #expect(tree.focused == c)
    tree.focus(b)
    tree.focusNext()
    #expect(tree.focused == c)
    tree.focus(99)
    #expect(tree.focused == c)
}

@MainActor
@Test func aPoisonedTerminalLeavesItsSiblingPanesRunning() throws {
    let sick = try TerminalSession(cols: 10, rows: 3)
    let well = try TerminalSession(cols: 10, rows: 3)
    #expect(!sick.isPoisoned)
    #expect(tt_term_test_panic(sick.term) == TT_PANIC)
    // Any later call finds out, and the first one to look reports a change
    // so the view redraws its notice.
    #expect(sick.update())
    #expect(sick.isPoisoned)
    #expect(!sick.update())
    #expect(sick.feed(Array("x".utf8)) == TT_POISONED)
    #expect(sick.resize(cols: 5, rows: 3, widthPx: 0, heightPx: 0) == TT_POISONED)

    #expect(well.feed(Array("ok".utf8)) == TT_OK)
    #expect(well.update())
    #expect(!well.isPoisoned)
    #expect(well.runs(row: 0).map(\.text) == ["ok"])
    #expect(well.resize(cols: 5, rows: 3, widthPx: 0, heightPx: 0) == TT_OK)
}

@MainActor
@Test func hostKeepsOneTerminalViewPerPaneThroughSplitsAndCloses() {
    let host = PaneHostView(frame: NSRect(x: 0, y: 0, width: 400, height: 300))
    #expect(host.paneCount == 1)
    host.splitRight(nil)
    host.splitDown(nil)
    #expect(host.paneCount == 3)
    host.closePane(nil)
    host.focusPreviousPane(nil)
    host.closePane(nil)
    #expect(host.paneCount == 1)
}
