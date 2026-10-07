// The damage of every frame update since the last draw, composed. The
// core reports damage per `tt_frame_update`, but the view may update
// several times (output, a key, a resize) before it draws once.

import CScull

public struct FrameDamage: Equatable, Sendable {
    /// For each row of the latest frame, the row of the last drawn picture
    /// it still shows unchanged, or nil when it must be repainted. Empty
    /// until the first update: everything is to be painted.
    public private(set) var sources: [Int?] = []

    public init() {}

    /// Nothing changed since a drawn picture of `rows` rows.
    init(clean rows: Int) { sources = Array(0..<rows) }

    mutating func absorb(_ view: tt_frame_view) {
        guard view.updated != 0 else { return }
        let rows = Int(view.rows)
        guard view.full == 0, rows == sources.count else {
            sources = Array(repeating: nil, count: rows)
            return
        }
        // Every scroll reads the picture as it was before this update.
        let before = sources
        for scroll in UnsafeBufferPointer(start: view.scrolls, count: view.scrolls_len) {
            let (start, end, from) = (Int(scroll.start), min(Int(scroll.end), rows), Int(scroll.from))
            for row in start..<max(start, end) {
                let source = from + row - start
                sources[row] = source < before.count ? before[source] : nil
            }
        }
        for row in UnsafeBufferPointer(start: view.dirty, count: view.dirty_len) where Int(row) < rows {
            sources[Int(row)] = nil
        }
    }
}
