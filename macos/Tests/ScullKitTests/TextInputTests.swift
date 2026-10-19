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

/// Marked text as an input method gives it: `thick` underlines the clause
/// being converted, `tagged` carries the clause segment.
private func marked(_ parts: [(String, thick: Bool, tagged: Bool)]) -> NSAttributedString {
    let text = NSMutableAttributedString()
    for (index, part) in parts.enumerated() {
        var attributes: [NSAttributedString.Key: Any] = [.underlineStyle: part.thick ? NSUnderlineStyle.thick.rawValue : NSUnderlineStyle.single.rawValue]
        if part.tagged { attributes[.markedClauseSegment] = index }
        text.append(NSAttributedString(string: part.0, attributes: attributes))
    }
    return text
}

@Test func clausesComeFromTheInputMethodsAttributes() {
    var input = TextInput()
    // Converting: the thick clause is the target, the tagged others are converted.
    input.setMarked(marked([("今日", false, true), ("は", true, true), ("晴れ", false, true)]),
                    selectedRange: NSRange(location: 2, length: 1))
    #expect(input.preedit.clauses == [
        PreeditClause(start: 0, end: 6, kind: UInt8(TT_PREEDIT_CONVERTED)),
        PreeditClause(start: 6, end: 9, kind: UInt8(TT_PREEDIT_TARGET)),
        PreeditClause(start: 9, end: 15, kind: UInt8(TT_PREEDIT_CONVERTED)),
    ])
    // Typing: tagged but nothing thick is still raw input.
    input.setMarked(marked([("にほん", false, true)]), selectedRange: NSRange(location: 3, length: 0))
    #expect(input.preedit.clauses == [PreeditClause(start: 0, end: 9, kind: UInt8(TT_PREEDIT_INPUT))])
    // Offsets are UTF-8 even where UTF-16 differs: a surrogate pair is four bytes.
    input.setMarked(marked([("😀", false, true), ("a", true, true)]), selectedRange: NSRange(location: 3, length: 0))
    #expect(input.preedit.clauses == [
        PreeditClause(start: 0, end: 4, kind: UInt8(TT_PREEDIT_CONVERTED)),
        PreeditClause(start: 4, end: 5, kind: UInt8(TT_PREEDIT_TARGET)),
    ])
}

@Test func textTheInputMethodDidNotMarkHasNoClauses() {
    var input = TextInput()
    input.setMarked("にほ", selectedRange: NSRange(location: 2, length: 0))
    #expect(input.preedit.clauses.isEmpty)
    // Only an underline, thin, and no segment tags: nothing to tell apart.
    input.setMarked(marked([("ab", false, false)]), selectedRange: NSRange(location: 2, length: 0))
    #expect(input.preedit.clauses.isEmpty)
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

@MainActor
@Test func theClausesReachTheFrameAsSpans() throws {
    let session = try TerminalSession(cols: 10, rows: 1)
    let clauses = [PreeditClause(start: 0, end: 3, kind: UInt8(TT_PREEDIT_CONVERTED)),
                   PreeditClause(start: 3, end: 9, kind: UInt8(TT_PREEDIT_TARGET))]
    #expect(session.preedit("は日本", caret: 3, clauses: clauses) == TT_OK)
    #expect(session.update())
    let view = session.view
    let spans = Array(UnsafeBufferPointer(start: view.preedit_spans, count: view.preedit_spans_len))
    #expect(spans.map { [Int($0.col), Int($0.cols), Int($0.kind)] }
            == [[0, 2, Int(TT_PREEDIT_CONVERTED)], [2, 4, Int(TT_PREEDIT_TARGET)]])
    // Without clauses the frame has the one span of the preedit and no kinds.
    #expect(session.preedit("は日本", caret: 3) == TT_OK)
    #expect(session.update())
    #expect(session.view.preedit_spans_len == 0 && session.view.preedit.cols == 6)
}

@MainActor
@Test func theViewAsksInputMethodsForTheirClauseAttributes() throws {
    let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 100))
    #expect(view.validAttributesForMarkedText().contains(.markedClauseSegment))
    #expect(view.validAttributesForMarkedText().contains(.underlineStyle))
    view.session = try TerminalSession(cols: 20, rows: 3)
    view.setMarkedText(marked([("今日", false, true), ("は", true, true)]),
                       selectedRange: NSRange(location: 2, length: 0), replacementRange: none)
    let frame = try #require(view.session?.view)
    #expect(frame.preedit_spans_len == 2)
    #expect(frame.preedit_spans[1].kind == UInt8(TT_PREEDIT_TARGET))
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
