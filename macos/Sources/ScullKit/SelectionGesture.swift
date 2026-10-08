// The pure half of mouse selection: which viewport cell a point is over,
// and what a press does. TerminalView+Selection runs the result.

import AppKit
import CScull

enum SelectionGesture {
    /// What a left press does to the selection.
    enum Press: Equatable {
        case start(kind: Int32)
        case extend
    }

    /// The viewport cell under `point` (in view points, top-left origin),
    /// clamped to the grid.
    static func cell(at point: CGPoint, cellWidth: CGFloat, cellHeight: CGFloat,
                     cols: Int, rows: Int) -> (row: Int, col: Int) {
        let col = min(max(0, Int(point.x / cellWidth)), max(0, cols - 1))
        let row = min(max(0, Int(point.y / cellHeight)), max(0, rows - 1))
        return (row, col)
    }

    /// Shift asks for the host's selection even while the program tracks
    /// the mouse, as in xterm.
    static func forcesSelection(_ flags: NSEvent.ModifierFlags) -> Bool { flags.contains(.shift) }

    /// Two clicks take a word, three a line; Option drags a block. Shift
    /// grows a selection that is there, or starts one when none is.
    static func press(clickCount: Int, flags: NSEvent.ModifierFlags, hasSelection: Bool) -> Press {
        if flags.contains(.shift), hasSelection { return .extend }
        switch clickCount {
        case 2: return .start(kind: TT_SELECT_WORD)
        case 3...: return .start(kind: TT_SELECT_LINE)
        default: return .start(kind: flags.contains(.option) ? TT_SELECT_BLOCK : TT_SELECT_CELL)
        }
    }
}
