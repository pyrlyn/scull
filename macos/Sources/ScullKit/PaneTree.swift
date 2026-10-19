// The layout of one window's panes as a value: a tree whose leaves are
// panes and whose nodes split the space in two. No views and no core
// handles live here, so the same shape can be tested without a screen and
// mirrored on Windows, where the panes become viewports of one swap chain.

public typealias PaneID = Int

public enum SplitAxis: Equatable, Sendable {
    /// The panes sit side by side.
    case sideBySide
    /// The panes sit one above the other.
    case stacked
}

public indirect enum PaneNode: Equatable, Sendable {
    case leaf(PaneID)
    case split(SplitAxis, PaneNode, PaneNode)

    /// The panes in reading order: left to right, top to bottom.
    public var leaves: [PaneID] {
        switch self {
        case .leaf(let id): [id]
        case .split(_, let first, let second): first.leaves + second.leaves
        }
    }

    /// The subtree without `id`; nil when `id` was the only pane.
    fileprivate func removing(_ id: PaneID) -> PaneNode? {
        switch self {
        case .leaf(let leaf): leaf == id ? nil : self
        case .split(let axis, let first, let second):
            switch (first.removing(id), second.removing(id)) {
            // The sibling takes the closed pane's whole share.
            case (nil, let rest), (let rest, nil): rest
            case (let a?, let b?): .split(axis, a, b)
            }
        }
    }

    fileprivate func splitting(_ id: PaneID, axis: SplitAxis, new: PaneID) -> PaneNode {
        switch self {
        case .leaf(let leaf): leaf == id ? .split(axis, self, .leaf(new)) : self
        case .split(let a, let first, let second):
            .split(a, first.splitting(id, axis: axis, new: new), second.splitting(id, axis: axis, new: new))
        }
    }
}

public struct PaneTree: Equatable, Sendable {
    public private(set) var root: PaneNode
    public private(set) var focused: PaneID
    private var nextID: PaneID

    public init() {
        root = .leaf(0)
        focused = 0
        nextID = 1
    }

    public var panes: [PaneID] { root.leaves }

    /// Splits the focused pane, puts the new pane after it and focuses it.
    @discardableResult
    public mutating func splitFocused(_ axis: SplitAxis) -> PaneID {
        let id = nextID
        nextID += 1
        root = root.splitting(focused, axis: axis, new: id)
        focused = id
        return id
    }

    /// Removes a pane. The pane after it, or the one before when it was
    /// last, takes the focus if it had it. False when `id` is not here or
    /// is the last pane; closing the last one is the window's job.
    @discardableResult
    public mutating func close(_ id: PaneID) -> Bool {
        let order = panes
        guard let index = order.firstIndex(of: id), order.count > 1, let rest = root.removing(id) else {
            return false
        }
        root = rest
        if focused == id { focused = order.indices.contains(index + 1) ? order[index + 1] : order[index - 1] }
        return true
    }

    /// Focuses `id` if it is a pane here.
    public mutating func focus(_ id: PaneID) {
        if panes.contains(id) { focused = id }
    }

    /// Moves the focus along the reading order, wrapping around.
    public mutating func focusNext(by step: Int = 1) {
        let order = panes
        guard let index = order.firstIndex(of: focused) else { return }
        focused = order[(index + step % order.count + order.count) % order.count]
    }
}
