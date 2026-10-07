// The view as an NSTextInputClient: input methods compose here (CJK,
// dead keys, the accent menu), the composing text goes to the core's
// frame as preedit so any renderer draws it, and committed text goes to
// the child as text. The state lives in TextInput.

import AppKit
import CScull

// AppKit calls these on the main thread; the protocol predates actors.
extension TerminalView: @preconcurrency NSTextInputClient {
    /// Runs `event` through the input method. The text it typed when the
    /// key is the core's to encode; nil when the input method took the
    /// key, or composed text, which is sent here.
    func interpret(_ event: NSEvent) -> String? {
        textInput.beginKey()
        interpretKeyEvents([event])
        switch textInput.endKey() {
        case .key(let text): return text
        case .text(let text): commit(text)
        case .consumed: break
        }
        return nil
    }

    private func commit(_ text: String) {
        if session?.text(text) == TT_FULL { NSSound.beep() }
        showPreedit()
    }

    private func showPreedit() {
        guard let session else { return }
        let (text, caret) = textInput.preedit
        _ = session.preedit(text, caret: caret)
        // Composing returns the view to the screen, where the cursor is.
        if !text.isEmpty { _ = session.scrollDisplay(Int32.min) }
        if session.update() { needsDisplay = true }
    }

    public func insertText(_ string: Any, replacementRange: NSRange) {
        guard let text = textInput.insert(string) else { return showPreedit() }
        commit(text)
    }

    public func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        textInput.setMarked(string, selectedRange: selectedRange)
        showPreedit()
    }

    // AppKit asks the view to accept the marked text as typed.
    public func unmarkText() {
        guard textInput.hasMarkedText else { return }
        insertText(textInput.marked.string, replacementRange: NSRange(location: NSNotFound, length: 0))
    }

    public func selectedRange() -> NSRange { textInput.selectedRange }

    public func markedRange() -> NSRange { textInput.markedRange }

    public func hasMarkedText() -> Bool { textInput.hasMarkedText }

    public func attributedSubstring(forProposedRange range: NSRange,
                                    actualRange: NSRangePointer?) -> NSAttributedString? {
        textInput.substring(range, actualRange: actualRange)
    }

    // The core draws the preedit in one style, so no attribute is used.
    public func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    /// The cursor cell, which the frame keeps at the composition's caret,
    /// in screen coordinates: the candidate window opens beside it.
    public func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        actualRange?.pointee = range
        guard let session, let window else { return .zero }
        let cursor = session.view.cursor
        let rect = NSRect(x: CGFloat(cursor.col) * cellWidth, y: CGFloat(cursor.row) * cellHeight,
                          width: cellWidth, height: cellHeight)
        return window.convertToScreen(convert(rect, to: nil))
    }

    public func characterIndex(for point: NSPoint) -> Int { NSNotFound }

    /// Underlines the composing text the frame placed on its row.
    func drawPreedit(in ctx: CGContext) {
        guard let preedit = session?.view.preedit, preedit.cols > 0 else { return }
        let thickness = max(1, font.underlineThickness)
        ctx.setFillColor(Palette.cgColor(palette.foreground))
        ctx.fill(CGRect(x: CGFloat(preedit.col) * cellWidth,
                        y: CGFloat(preedit.row + 1) * cellHeight - thickness,
                        width: CGFloat(preedit.cols) * cellWidth, height: thickness))
    }

    #if DEBUG
    /// Composes `-ScullPreedit` text, so a scripted snapshot shows it.
    func composeDebugPreedit() {
        guard let text = UserDefaults.standard.string(forKey: "ScullPreedit") else { return }
        setMarkedText(text, selectedRange: NSRange(location: (text as NSString).length, length: 0),
                      replacementRange: NSRange(location: NSNotFound, length: 0))
    }
    #endif
}
