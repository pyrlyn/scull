// The input method's side of the terminal view: the marked (composing)
// text, its selection, and what a key event turned into. Kept apart from
// the view so tests drive the NSTextInputClient logic without a window or
// a real input method.
//
// The terminal shows the input system no document of its own: only the
// marked text exists, so its ranges start at 0. What the child already
// printed is not editable text.

import AppKit
import CScull
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

/// A stretch of the marked text for the core's frame: a UTF-8 byte range
/// and a `TT_PREEDIT_*` kind.
struct PreeditClause: Equatable {
    var start: Int
    var end: Int
    var kind: UInt8
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
    /// the selection as a UTF-8 offset, and the clauses the input method
    /// marked.
    var preedit: (text: String, caret: Int, clauses: [PreeditClause]) {
        let text = marked.string
        return (text, Self.utf8Offset(of: selection.location, in: text), clauses(of: text))
    }

    /// What the input method says about each stretch of the marked text. A
    /// Japanese input method gives the clause being converted a thick
    /// underline and tags every clause with `NSMarkedClauseSegment`; the
    /// target is the thick one, the other tagged clauses are converted, and
    /// while nothing is thick nothing is being converted, so the text is
    /// still input. An input method that marks nothing gets none, and the
    /// core shows its text as one plain span.
    private func clauses(of text: String) -> [PreeditClause] {
        var runs: [(range: NSRange, thick: Bool, tagged: Bool)] = []
        marked.enumerateAttributes(in: NSRange(location: 0, length: marked.length)) { attributes, range, _ in
            let style = (attributes[.underlineStyle] as? Int) ?? 0
            runs.append((range, style & NSUnderlineStyle.thick.rawValue != 0, attributes[.markedClauseSegment] != nil))
        }
        guard runs.contains(where: { $0.thick || $0.tagged }) else { return [] }
        let converting = runs.contains { $0.thick }
        return runs.map { run in
            let kind = run.thick ? TT_PREEDIT_TARGET : (run.tagged && converting ? TT_PREEDIT_CONVERTED : TT_PREEDIT_INPUT)
            return PreeditClause(start: Self.utf8Offset(of: run.range.location, in: text),
                                 end: Self.utf8Offset(of: NSMaxRange(run.range), in: text), kind: UInt8(kind))
        }
    }

    /// The UTF-8 offset of a UTF-16 one; inside a surrogate pair it is the
    /// pair's start, as the core would place a caret there.
    private static func utf8Offset(of utf16Offset: Int, in text: String) -> Int {
        var (utf16, utf8) = (0, 0)
        for scalar in text.unicodeScalars {
            let next = utf16 + scalar.utf16.count
            guard next <= utf16Offset else { break }
            (utf16, utf8) = (next, utf8 + scalar.utf8.count)
        }
        return utf8
    }

    private static func plain(_ string: Any) -> String {
        (string as? NSAttributedString)?.string ?? (string as? String) ?? ""
    }
}
