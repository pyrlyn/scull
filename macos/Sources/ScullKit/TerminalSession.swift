// One tt_term and the tt_frame that reads it, on the main actor. Every
// tt_* call happens here and only on the main thread: the wakeup comes on
// a core thread, so it only posts to the main queue and the work runs
// there.

import CScull
import Foundation

public enum ScullError: Error, Equatable {
    case status(Int32)
    case noFrame
}

/// Carried through the C wakeup's userdata. The core coalesces wakeups
/// until the next poll, so each one just posts; nothing here touches
/// tt_* because the callback runs on a core thread.
private final class Waker: Sendable {
    let action: @MainActor @Sendable () -> Void
    init(_ action: @escaping @MainActor @Sendable () -> Void) { self.action = action }
    func fire() {
        DispatchQueue.main.async { [action] in MainActor.assumeIsolated(action) }
    }
}

@MainActor
public final class TerminalSession {
    private let term: OpaquePointer
    private let frame: OpaquePointer
    private let waker: Unmanaged<Waker>?
    public private(set) var view = tt_frame_view()

    /// Runs the user's shell. `onWake` runs on the main actor whenever
    /// there is output or an event to look at.
    public init(cols: UInt16, rows: UInt16, env: [String], cwd: String,
                onWake: @escaping @MainActor @Sendable () -> Void) throws(ScullError) {
        let waker = Unmanaged.passRetained(Waker(onWake))
        var out: OpaquePointer?
        let status = withStrings(env + [cwd]) { strs in
            var options = Self.options(cols: cols, rows: rows)
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
    public init(cols: UInt16, rows: UInt16) throws(ScullError) {
        var options = Self.options(cols: cols, rows: rows)
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

    private static func options(cols: UInt16, rows: UInt16) -> tt_term_options {
        var options = tt_term_options()
        options.struct_size = UInt32(MemoryLayout<tt_term_options>.size)
        options.abi_version = UInt32(TT_ABI_VERSION)
        options.cols = cols
        options.rows = rows
        options.scrollback = 10_000
        return options
    }

    /// Takes every pending event; true once the child has exited. Rings the
    /// bell through `bell`.
    public func drainEvents(bell: () -> Void) -> Bool {
        var event = tt_event()
        event.struct_size = UInt32(MemoryLayout<tt_event>.size)
        var exited = false
        while tt_term_poll_event(term, &event) == TT_OK {
            switch Int32(event.kind) {
            case TT_EVENT_BELL: bell()
            case TT_EVENT_CHILD_EXITED: exited = true
            default: break
            }
        }
        return exited
    }

    /// Refreshes `view`; true when it changed.
    @discardableResult
    public func update() -> Bool {
        var next = tt_frame_view()
        next.struct_size = UInt32(MemoryLayout<tt_frame_view>.size)
        guard tt_frame_update(frame, term, &next) == TT_OK else { return false }
        view = next
        return next.updated != 0
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
            return tt_term_key(term, &event)
        }
    }

    public func text(_ text: String) -> tt_status {
        var text = text
        return text.withUTF8 { tt_term_text(term, $0.baseAddress, $0.count) }
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
        return text.withUTF8 { tt_term_paste(term, $0.baseAddress, $0.count) }
    }

    public func focus(_ focused: Bool) -> tt_status { tt_term_focus(term, focused ? 1 : 0) }

    /// Sends a mouse event; true when the program took it.
    public func mouse(_ event: tt_mouse_event) -> Bool {
        var event = event
        event.struct_size = UInt32(MemoryLayout<tt_mouse_event>.size)
        var taken: UInt8 = 0
        return tt_term_mouse(term, &event, &taken) == TT_OK && taken != 0
    }

    public func scrollDisplay(_ delta: Int32) -> tt_status { tt_term_scroll_display(term, delta) }

    public func resizeBegin() -> tt_status { tt_term_resize_begin(term) }

    public func resize(cols: UInt16, rows: UInt16, widthPx: UInt16, heightPx: UInt16) -> tt_status {
        tt_term_resize(term, cols, rows, widthPx, heightPx)
    }

    public func feed(_ bytes: [UInt8]) -> tt_status {
        bytes.withUnsafeBufferPointer { tt_term_feed(term, $0.baseAddress, $0.count) }
    }
}

/// Calls `body` with the strings as tt_str values that stay valid for the
/// call; Swift gives no stable pointer to several strings at once.
private func withStrings<R>(_ strings: [String], _ body: (UnsafeBufferPointer<tt_str>) -> R) -> R {
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
