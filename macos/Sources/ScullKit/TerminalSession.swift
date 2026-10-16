// One tt_term and the tt_frame that reads it, on the main actor. Every
// tt_* call happens here and only on the main thread: the wakeup comes on
// a core thread, so it only posts to the main queue and the work runs
// there.

import AppKit
import CScull

public enum ScullError: Error, Equatable {
    case status(Int32)
    case noFrame
}

/// Carried through the C wakeup's userdata. The core coalesces wakeups
/// until the next poll, so each one just posts; nothing here touches
/// tt_* because the callback runs on a core thread.
final class Waker: Sendable {
    let action: @MainActor @Sendable () -> Void
    init(_ action: @escaping @MainActor @Sendable () -> Void) { self.action = action }
    func fire() {
        DispatchQueue.main.async { [action] in MainActor.assumeIsolated(action) }
    }
}

@MainActor
public final class TerminalSession {
    // Not private so tests can hand the raw handle to the core's test hook.
    let term: OpaquePointer
    private let frame: OpaquePointer
    private let waker: Unmanaged<Waker>?
    public private(set) var view = tt_frame_view()
    /// Damage of every update since the renderer last took it.
    private var damage = FrameDamage()
    /// True once a call answered `TT_POISONED` or `TT_PANIC`: the core hit a
    /// bug in this terminal, and only freeing it is left. Other sessions are
    /// untouched, so the pane shows a notice and its siblings keep running.
    public private(set) var isPoisoned = false

    /// The screen's text, read once per changed frame: an accessibility
    /// client asks for it many times per query.
    private var screenCache: ScreenText?

    /// The window title the program set last (OSC 0 or 2).
    public private(set) var title: String?
    /// The directory the shell reported last (OSC 7), as a path.
    public private(set) var workingDirectory: String?
    /// Where OSC 52 writes go; a test passes a private one.
    var pasteboard = NSPasteboard.general

    /// Runs the user's shell. `onWake` runs on the main actor whenever
    /// there is output or an event to look at.
    public init(cols: UInt16, rows: UInt16, env: [String], cwd: String, scrollback: UInt32 = 10_000,
                onWake: @escaping @MainActor @Sendable () -> Void) throws(ScullError) {
        let waker = Unmanaged.passRetained(Waker(onWake))
        var out: OpaquePointer?
        let status = withStrings(env + [cwd]) { strs in
            var options = Self.options(cols: cols, rows: rows, scrollback: scrollback)
            options.env = strs.baseAddress
            options.env_len = strs.count - 1
            options.cwd = strs[strs.count - 1]
            options.wakeup = { userdata in
                guard let userdata else { return }
                Unmanaged<Waker>.fromOpaque(userdata).takeUnretainedValue().fire()
            }
            options.userdata = waker.toOpaque()
            return tt_term_spawn(&options, &out)
        }
        guard status == TT_OK, let out else {
            waker.release()
            throw .status(status.rawValue)
        }
        guard let frame = tt_frame_new() else {
            tt_term_free(out)
            waker.release()
            throw .noFrame
        }
        (term, self.frame, self.waker) = (out, frame, waker)
    }

    /// A terminal with no child, fed by `feed`; for tests.
    public init(cols: UInt16, rows: UInt16, scrollback: UInt32 = 10_000) throws(ScullError) {
        var options = Self.options(cols: cols, rows: rows, scrollback: scrollback)
        var out: OpaquePointer?
        let status = tt_term_new(&options, &out)
        guard status == TT_OK, let out else { throw .status(status.rawValue) }
        guard let frame = tt_frame_new() else {
            tt_term_free(out)
            throw .noFrame
        }
        (term, self.frame, waker) = (out, frame, nil)
    }

    isolated deinit {
        tt_frame_free(frame)
        // Joins the core's threads, so no wakeup can follow the release.
        tt_term_free(term)
        waker?.release()
    }

    private static func options(cols: UInt16, rows: UInt16, scrollback: UInt32) -> tt_term_options {
        var options = tt_term_options()
        options.struct_size = UInt32(MemoryLayout<tt_term_options>.size)
        options.abi_version = UInt32(TT_ABI_VERSION)
        options.cols = cols
        options.rows = rows
        options.scrollback = scrollback
        return options
    }

    @discardableResult
    private func track(_ status: tt_status) -> tt_status {
        if status == TT_POISONED || status == TT_PANIC { isPoisoned = true }
        return status
    }

    /// Takes every pending event; true once the child has exited. Rings the
    /// bell through `bell`, keeps the title and directory, and writes OSC 52
    /// clipboard writes to `pasteboard`.
    public func drainEvents(bell: () -> Void) -> Bool {
        var event = tt_event()
        event.struct_size = UInt32(MemoryLayout<tt_event>.size)
        var exited = false
        while track(tt_term_poll_event(term, &event)) == TT_OK {
            switch Int32(event.kind) {
            case TT_EVENT_BELL: bell()
            case TT_EVENT_CHILD_EXITED: exited = true
            case TT_EVENT_TITLE where Int32(event.detail) != TT_TITLE_ICON:
                if let bytes = eventText(event) { title = String(decoding: bytes, as: UTF8.self) }
            case TT_EVENT_WORKING_DIRECTORY:
                // Only a local file URL names a directory this Mac can open.
                let url = eventText(event).flatMap { URL(string: String(decoding: $0, as: UTF8.self)) }
                if let url, url.isFileURL { workingDirectory = url.path }
            case TT_EVENT_CLIPBOARD_WRITE:
                // The child's bytes need not be text; anything else is dropped.
                if let bytes = eventText(event), let text = String(bytes: bytes, encoding: .utf8) {
                    pasteboard.clearContents()
                    pasteboard.setString(text, forType: .string)
                }
            case TT_EVENT_CLIPBOARD_READ:
                // A read hands the user's clipboard to whatever the shell
                // runs, a remote host included; refused until a setting
                // lets the user allow it. The answer keeps it from waiting.
                track(tt_term_clipboard_deny(term, event.id))
            default: break
            }
        }
        return exited
    }

    /// The text of `event`, the one polled last, sized from the event.
    private func eventText(_ event: tt_event) -> [UInt8]? {
        let count = Int(event.text_len)
        var bytes = [UInt8](repeating: 0, count: count)
        var len = 0
        let status = bytes.withUnsafeMutableBufferPointer {
            track(tt_term_event_text(term, event.serial, UInt32(TT_EVENT_TEXT_BODY), $0.baseAddress, count, &len))
        }
        return status == TT_OK ? Array(bytes.prefix(len)) : nil
    }

    /// Refreshes `view`; true when it changed.
    @discardableResult
    public func update() -> Bool {
        var next = tt_frame_view()
        next.struct_size = UInt32(MemoryLayout<tt_frame_view>.size)
        let wasPoisoned = isPoisoned
        guard track(tt_frame_update(frame, term, &next)) == TT_OK else {
            // The notice replaces the grid, so the view has to redraw once.
            return isPoisoned && !wasPoisoned
        }
        view = next
        damage.absorb(next)
        if next.updated != 0 { screenCache = nil }
        return next.updated != 0
    }

    /// The damage since the last call, composed over every update.
    public func takeDamage() -> FrameDamage {
        defer { damage = FrameDamage(clean: Int(view.rows)) }
        return damage
    }

    /// The viewport's text, a line per row (see `tt_term_read_text`).
    func screenText() -> ScreenText {
        if let screenCache { return screenCache }
        let read = ScreenText(readText() ?? "")
        screenCache = read
        return read
    }

    private func readText() -> String? {
        sizedText { tt_term_read_text(term, 0, UInt16.max, $0, $1, $2) }
    }

    /// Text from a sized-buffer export: asks for the length, then reads.
    private func sizedText(_ read: (UnsafeMutablePointer<UInt8>?, Int, UnsafeMutablePointer<Int>) -> tt_status)
        -> String? {
        var cap = 0
        // The child can print between asking for the length and reading,
        // so a few tries; past them the reader gets nothing this frame.
        for _ in 0..<4 {
            var bytes = [UInt8](repeating: 0, count: cap)
            var len = 0
            let status = bytes.withUnsafeMutableBufferPointer { read($0.baseAddress, cap, &len) }
            if status == TT_OK { return String(decoding: bytes.prefix(len), as: UTF8.self) }
            guard status == TT_FULL else { return nil }
            cap = len + len / 8
        }
        return nil
    }

    // Selection and search live in the core, anchored to its history; the
    // frame marks their cells. Rows and columns are viewport cells.

    public func select(_ kind: Int32, row: Int, col: Int) -> tt_status {
        track(tt_term_select_start(term, UInt32(kind), UInt16(clamping: row), UInt16(clamping: col)))
    }

    public func extendSelection(row: Int, col: Int) -> tt_status {
        track(tt_term_select_extend(term, UInt16(clamping: row), UInt16(clamping: col)))
    }

    public func clearSelection() -> tt_status { track(tt_term_select_clear(term)) }

    /// The selected text; nil when nothing is selected.
    public func selectionText() -> String? {
        sizedText { track(tt_term_selection_text(term, $0, $1, $2)) }
    }

    /// Output that rewrites the selected rows drops the selection, so the
    /// core is asked rather than the host remembering.
    public var hasSelection: Bool {
        var len = 0
        return tt_term_selection_text(term, nil, 0, &len) != TT_EMPTY
    }

    /// Replaces the search; an empty pattern ends it.
    public func search(_ pattern: String, ignoreCase: Bool) -> tt_status {
        var pattern = pattern
        let flags = ignoreCase ? UInt32(TT_SEARCH_IGNORE_CASE) : 0
        return pattern.withUTF8 { track(tt_term_search_set(term, tt_str(ptr: $0.baseAddress, len: $0.count), flags)) }
    }

    /// Moves to the next or previous match and scrolls to it; nil when
    /// there is none.
    public func searchStep(forward: Bool) -> tt_match? {
        var found = tt_match()
        found.struct_size = UInt32(MemoryLayout<tt_match>.size)
        return track(tt_term_search_step(term, forward ? 1 : 0, &found)) == TT_OK ? found : nil
    }

    /// Matches in the whole history, up to `TT_MAX_SEARCH_MATCHES`. It
    /// rescans everything, so it is not for every keystroke.
    public func searchCount() -> Int {
        var count = 0
        return track(tt_term_search_count(term, &count)) == TT_OK ? count : 0
    }

    public func cell(row: Int, col: Int) -> tt_cell? {
        let index = row * Int(view.cols) + col
        guard col < view.cols, index >= 0, index < view.cells_len else { return nil }
        return view.cells[index]
    }

    public func style(_ index: UInt16) -> tt_style {
        Int(index) < view.styles_len ? view.styles[Int(index)] : tt_style()
    }

    /// The runs of a row with their text; the text slices are checked,
    /// since the core's output is still data from the child.
    public func runs(row: Int) -> [(run: tt_run, text: String)] {
        guard row >= 0, row < view.lines_len else { return [] }
        let line = view.lines[row]
        return (0..<line.runs_len).compactMap { i in
            let run = line.runs[i]
            let end = Int(run.text_start) + Int(run.text_len)
            guard end <= line.text_len else { return nil }
            let bytes = UnsafeBufferPointer(start: line.text + Int(run.text_start), count: Int(run.text_len))
            return (run, String(decoding: bytes, as: UTF8.self))
        }
    }

    // Input. Each answers the tt_status; TT_FULL means the child is not
    // reading and nothing was sent.

    public func key(_ event: tt_key_event, text: String) -> tt_status {
        var event = event
        event.struct_size = UInt32(MemoryLayout<tt_key_event>.size)
        var text = text
        return text.withUTF8 { bytes in
            event.text = tt_str(ptr: bytes.baseAddress, len: bytes.count)
            return track(tt_term_key(term, &event))
        }
    }

    public func text(_ text: String) -> tt_status {
        var text = text
        return text.withUTF8 { track(tt_term_text(term, $0.baseAddress, $0.count)) }
    }

    /// Shows an input method's composing text at the cursor from the next
    /// update, the caret at UTF-8 offset `caret`; empty text clears it.
    /// It never reaches the child.
    public func preedit(_ text: String, caret: Int) -> tt_status {
        var text = text
        return text.withUTF8 { tt_frame_preedit(frame, $0.baseAddress, $0.count, max(0, caret)) }
    }

    public func paste(_ text: String) -> tt_status {
        var text = text
        return text.withUTF8 { track(tt_term_paste(term, $0.baseAddress, $0.count)) }
    }

    public func focus(_ focused: Bool) -> tt_status { track(tt_term_focus(term, focused ? 1 : 0)) }

    /// Sends a mouse event; true when the program took it. A program that
    /// tracks the mouse owns the event even when it could not be written,
    /// so the host never selects under it.
    public func mouse(_ event: tt_mouse_event) -> Bool {
        var event = event
        event.struct_size = UInt32(MemoryLayout<tt_mouse_event>.size)
        var taken: UInt8 = 0
        track(tt_term_mouse(term, &event, &taken))
        return taken != 0
    }

    public func scrollDisplay(_ delta: Int32) -> tt_status { track(tt_term_scroll_display(term, delta)) }

    public func resizeBegin() -> tt_status { track(tt_term_resize_begin(term)) }

    public func resize(cols: UInt16, rows: UInt16, widthPx: UInt16, heightPx: UInt16) -> tt_status {
        track(tt_term_resize(term, cols, rows, widthPx, heightPx))
    }

    public func feed(_ bytes: [UInt8]) -> tt_status {
        bytes.withUnsafeBufferPointer { track(tt_term_feed(term, $0.baseAddress, $0.count)) }
    }
}

/// Calls `body` with the strings as tt_str values that stay valid for the
/// call; Swift gives no stable pointer to several strings at once.
func withStrings<R>(_ strings: [String], _ body: (UnsafeBufferPointer<tt_str>) -> R) -> R {
    let copies = strings.map { string -> (UnsafeMutablePointer<UInt8>, Int) in
        let utf8 = Array(string.utf8)
        let copy = UnsafeMutablePointer<UInt8>.allocate(capacity: max(utf8.count, 1))
        copy.initialize(from: utf8, count: utf8.count)
        return (copy, utf8.count)
    }
    defer { copies.forEach { $0.0.deallocate() } }
    let strs = copies.map { tt_str(ptr: UnsafePointer($0.0), len: $0.1) }
    return strs.withUnsafeBufferPointer(body)
}
