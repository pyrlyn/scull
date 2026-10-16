import AppKit
import CScull
import Testing
@testable import ScullKit

private let none = NSRange(location: NSNotFound, length: 0)

@Test func markedRangesFollowTheComposition() {
    var input = TextInput()
    #expect(!input.hasMarkedText)
    #expect(input.markedRange == none)
    #expect(input.selectedRange == NSRange(location: 0, length: 0))
    input.setMarked("にほ", selectedRange: NSRange(location: 2, length: 0))
    #expect(input.markedRange == NSRange(location: 0, length: 2))
    #expect(input.selectedRange == NSRange(location: 2, length: 0))
    #expect(input.preedit.text == "にほ" && input.preedit.caret == 6)
    // A converted clause: the caret goes to its start.
    input.setMarked(NSAttributedString(string: "日本語"), selectedRange: NSRange(location: 2, length: 1))
    #expect(input.preedit.caret == 6)
    // Ranges from the input method are clamped to the marked text.
    input.setMarked("ab", selectedRange: NSRange(location: 9, length: 4))
    #expect(input.selectedRange == NSRange(location: 2, length: 0))
    input.setMarked("ab", selectedRange: none)
    #expect(input.selectedRange == NSRange(location: 2, length: 0))
}

@Test func theCaretNeverSplitsASurrogatePair() {
    var input = TextInput()
    input.setMarked("a😀b", selectedRange: NSRange(location: 2, length: 0))
    #expect(input.preedit.caret == 1)
    input.setMarked("a😀b", selectedRange: NSRange(location: 3, length: 0))
    #expect(input.preedit.caret == 5)
}

@Test func substringsComeFromTheMarkedTextOnly() {
    var input = TextInput()
    var actual = none
    #expect(input.substring(NSRange(location: 0, length: 1), actualRange: &actual) == nil)
    input.setMarked("かな", selectedRange: NSRange(location: 2, length: 0))
    #expect(input.substring(NSRange(location: 1, length: 5), actualRange: &actual)?.string == "な")
    #expect(actual == NSRange(location: 1, length: 1))
    #expect(input.substring(none, actualRange: nil) == nil)
    #expect(input.substring(NSRange(location: 2, length: 1), actualRange: nil) == nil)
}

@Test func insertTextAfterPreeditCommitsItAsText() {
    var input = TextInput()
    input.beginKey()
    input.setMarked("にほん", selectedRange: NSRange(location: 3, length: 0))
    #expect(input.endKey() == .consumed)
    input.beginKey()
    #expect(input.insert(NSAttributedString(string: "日本")) == nil)
    #expect(input.endKey() == .text("日本"))
    #expect(!input.hasMarkedText && input.preedit.text.isEmpty)
    // Outside a key event (the character viewer, dictation) it is sent now.
    #expect(input.insert("→") == "→")
}

@Test func aDeadKeyComposesWithTheNextKey() {
    var input = TextInput()
    input.beginKey()
    input.setMarked("´", selectedRange: NSRange(location: 1, length: 0))
    #expect(input.endKey() == .consumed)
    input.beginKey()
    #expect(input.insert("é") == nil)
    #expect(input.endKey() == .text("é"))
    // Backspace inside a composition edits it and reaches no program.
    input.beginKey()
    input.setMarked("´", selectedRange: NSRange(location: 1, length: 0))
    _ = input.endKey()
    input.beginKey()
    input.setMarked("", selectedRange: NSRange(location: 0, length: 0))
    #expect(input.endKey() == .consumed)
}

@Test func aPlainKeyStaysAKey() {
    var input = TextInput()
    input.beginKey()
    #expect(input.insert("a") == nil)
    #expect(input.endKey() == .key("a"))
    input.beginKey()
    #expect(input.endKey() == .key(""))
}

@MainActor
@Test func thePreeditShowsInTheFrameAtTheCursor() throws {
    let session = try TerminalSession(cols: 10, rows: 2)
    #expect(session.feed(Array("ab".utf8)) == TT_OK)
    session.update()
    #expect(session.preedit("日本", caret: 6) == TT_OK)
    #expect(session.update())
    let preedit = session.view.preedit
    #expect(preedit.row == 0 && preedit.col == 2 && preedit.cols == 4)
    #expect(session.runs(row: 0).map(\.text) == ["ab", "日本"])
    #expect(session.cell(row: 0, col: 2)?.width == 2)
    #expect(session.view.cursor.col == 6)
    #expect(session.preedit("", caret: 0) == TT_OK)
    #expect(session.update())
    #expect(session.view.preedit.cols == 0)
    #expect(session.runs(row: 0).map(\.text) == ["ab"])
    #expect(session.view.cursor.col == 2)
}

private func key(_ chars: String, _ plain: String, code: UInt16, _ flags: NSEvent.ModifierFlags = []) -> NSEvent {
    NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: 0,
                     context: nil, characters: chars, charactersIgnoringModifiers: plain,
                     isARepeat: false, keyCode: code)!
}

@MainActor
@Test func theViewComposesThroughAppKitsInputSystem() throws {
    let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 100))
    // A terminal with no child, so nothing typed leaves the test.
    view.session = try TerminalSession(cols: 20, rows: 3)
    let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView = view
    view.setMarkedText("にほ", selectedRange: NSRange(location: 2, length: 0), replacementRange: none)
    #expect(view.hasMarkedText() && view.markedRange() == NSRange(location: 0, length: 2))
    #expect(view.session?.view.preedit.cols == 4)
    #expect(view.session?.runs(row: 0).map(\.text) == ["にほ"])
    // The candidate window opens at the caret, after the two wide cells.
    let rect = view.firstRect(forCharacterRange: view.selectedRange(), actualRange: nil)
    let caret = window.convertToScreen(view.convert(NSRect(x: 4 * view.cellWidth, y: 0, width: view.cellWidth,
                                                           height: view.cellHeight), to: nil))
    #expect(rect == caret)
    view.unmarkText()
    #expect(!view.hasMarkedText() && view.session?.view.preedit.cols == 0)
    // A plain key goes through the input context and comes back as a key.
    #expect(view.interpret(key("a", "a", code: 0)) == "a")
}
