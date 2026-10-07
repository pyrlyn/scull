// The input method's side of the terminal view: the marked (composing)
// text, its selection, and what a key event turned into. Kept apart from
// the view so tests drive the NSTextInputClient logic without a window or
// a real input method.
//
// The terminal shows the input system no document of its own: only the
// marked text exists, so its ranges start at 0. What the child already
// printed is not editable text.

import Foundation

/// What a key event becomes once the input method has seen it.
enum KeyOutcome: Equatable {
    /// The key is the core's to encode, with the text the layout typed.
    case key(String)
    /// An input method composed this text (a dead key, a CJK commit); it
    /// goes to the child as text, not as the key that finished it.
    case text(String)
    /// The input method took the key to edit its composition.
    case consumed
}

struct TextInput {
    private(set) var marked = NSAttributedString()
    /// The input method's selection inside the marked text, UTF-16.
    private(set) var selection = NSRange(location: 0, length: 0)
    /// Text inserted during the key event being interpreted; nil outside
    /// one, when insertions go straight to the child.
    private var typed: String?
    private var composedBefore = false

    var hasMarkedText: Bool { marked.length > 0 }

    var markedRange: NSRange {
        hasMarkedText ? NSRange(location: 0, length: marked.length) : NSRange(location: NSNotFound, length: 0)
    }

    // An empty document still has an insertion point; input methods anchor
    // to it, so this is never NSNotFound.
    var selectedRange: NSRange { hasMarkedText ? selection : NSRange(location: 0, length: 0) }

    mutating func beginKey() {
        typed = ""
        composedBefore = hasMarkedText
    }

    mutating func endKey() -> KeyOutcome {
        let text = typed ?? ""
        typed = nil
        guard composedBefore || hasMarkedText else { return .key(text) }
        return text.isEmpty ? .consumed : .text(text)
    }

    /// Commits `string` and ends the composition. Answers the text when it
    /// must be sent now; inside a key event it is kept for `endKey`.
    mutating func insert(_ string: Any) -> String? {
        let text = Self.plain(string)
        marked = NSAttributedString()
        selection = NSRange(location: 0, length: 0)
        guard let before = typed else { return text }
        typed = before + text
        return nil
    }

    mutating func setMarked(_ string: Any, selectedRange: NSRange) {
        marked = (string as? NSAttributedString) ?? NSAttributedString(string: Self.plain(string))
        let length = marked.length
        let location = selectedRange.location == NSNotFound ? length : min(max(0, selectedRange.location), length)
        selection = NSRange(location: location, length: min(max(0, selectedRange.length), length - location))
    }

    func substring(_ range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? {
        let whole = NSRange(location: 0, length: marked.length)
        guard range.location != NSNotFound, let part = range.intersection(whole), part.length > 0 else {
            return nil
        }
        actualRange?.pointee = part
        return marked.attributedSubstring(from: part)
    }

    /// The marked text for the core's frame, with the caret at the start of
    /// the selection as a UTF-8 offset.
    var preedit: (text: String, caret: Int) {
        let text = marked.string
        var (utf16, utf8) = (0, 0)
        for scalar in text.unicodeScalars {
            let next = utf16 + scalar.utf16.count
            guard next <= selection.location else { break }
            (utf16, utf8) = (next, utf8 + scalar.utf8.count)
        }
        return (text, utf8)
    }

    private static func plain(_ string: Any) -> String {
        (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
    }
}
