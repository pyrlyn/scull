// Mouse selection and Copy. The core keeps the selection and the frame
// marks it; the view only turns presses and drags into core calls.

import AppKit
import CScull

/// A left-button gesture the host took for selection.
struct SelectionDrag {
    var cell: (row: Int, col: Int)
    /// A single click that never moved selects nothing when released.
    var clearsOnRelease: Bool
}

extension TerminalView: NSMenuItemValidation {
    func selectionPress(_ event: NSEvent) {
        guard let session else { return }
        let cell = cell(at: convert(event.locationInWindow, from: nil))
        let press = SelectionGesture.press(clickCount: event.clickCount, flags: event.modifierFlags,
                                           hasSelection: session.hasSelection)
        switch press {
        case .extend: _ = session.extendSelection(row: cell.row, col: cell.col)
        case .start(let kind): _ = session.select(kind, row: cell.row, col: cell.col)
        }
        selectionDrag = SelectionDrag(cell: cell, clearsOnRelease: press == .start(kind: TT_SELECT_CELL)
                                          || press == .start(kind: TT_SELECT_BLOCK))
        refreshSelection()
    }

    func selectionDragged(_ event: NSEvent) {
        guard let session, var drag = selectionDrag else { return }
        let point = convert(event.locationInWindow, from: nil)
        // Past an edge the history scrolls a row per motion event; the
        // selection is anchored to the history, so it grows with it.
        let scroll: Int32 = point.y < 0 ? 1 : point.y >= bounds.height ? -1 : 0
        if scroll != 0 { _ = session.scrollDisplay(scroll) }
        let cell = cell(at: point)
        guard scroll != 0 || cell != drag.cell else { return }
        _ = session.extendSelection(row: cell.row, col: cell.col)
        (drag.cell, drag.clearsOnRelease) = (cell, false)
        selectionDrag = drag
        refreshSelection()
    }

    func selectionRelease() {
        if selectionDrag?.clearsOnRelease == true {
            _ = session?.clearSelection()
            refreshSelection()
        }
        selectionDrag = nil
    }

    private func refreshSelection() {
        if session?.update() == true { needsDisplay = true }
    }

    @objc public func copy(_ sender: Any?) {
        // The session's pasteboard, where OSC 52 writes go too.
        guard let session, let text = session.selectionText() else { return }
        session.pasteboard.clearContents()
        session.pasteboard.setString(text, forType: .string)
    }

    public func validateMenuItem(_ item: NSMenuItem) -> Bool {
        switch item.action {
        case #selector(copy(_:)): session?.hasSelection ?? false
        case #selector(findNext(_:)), #selector(findPrevious(_:)): find.canStep
        default: true
        }
    }
}
