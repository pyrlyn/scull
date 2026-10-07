// One window's panes: a PaneTree laid out with nested NSSplitViews, one
// TerminalView (and so one core terminal) per pane. The views live here
// by pane id, not inside the split views, so a split or a close only
// re-parents them and never restarts a shell. A crashed terminal poisons
// its own pane; its siblings share nothing with it.

import AppKit

/// Starts its divider in the middle: NSSplitView leaves new subviews at
/// whatever sizes they were added with.
private final class EvenSplitView: NSSplitView {
    private var placed = false

    override func layout() {
        super.layout()
        guard !placed, arrangedSubviews.count == 2 else { return }
        let length = isVertical ? bounds.width : bounds.height
        guard length > 0 else { return }
        placed = true
        setPosition((length - dividerThickness) / 2, ofDividerAt: 0)
    }
}

public final class PaneHostView: NSView {
    /// A New Tab command sets this to the window the next new window joins
    /// as a tab. SwiftUI's WindowGroup opens windows but cannot say where.
    public static weak var tabAnchor: NSWindow?

    private var tree = PaneTree()
    private var panes: [PaneID: TerminalView] = [:]
    private var rebuilding = false

    public override init(frame: NSRect) {
        super.init(frame: frame)
        addPane(tree.focused)
        rebuild()
        #if DEBUG
        if UserDefaults.standard.bool(forKey: "ScullSplit") { splitRight(nil) }
        if let path = UserDefaults.standard.string(forKey: "ScullHostSnapshot") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 10) { [weak self] in self?.snapshot(to: path) }
        }
        #endif
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    public override var acceptsFirstResponder: Bool { false }

    var paneCount: Int { panes.count }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        guard let window else { return }
        if let anchor = Self.tabAnchor, anchor !== window {
            Self.tabAnchor = nil
            anchor.addTabbedWindow(window, ordered: .above)
            window.makeKeyAndOrderFront(nil)
        }
        focusCurrent()
    }

    #if DEBUG
    // Both panes in one PNG, drawn by the view itself like TerminalView's.
    private func snapshot(to path: String) {
        guard let rep = bitmapImageRepForCachingDisplay(in: bounds) else { return }
        cacheDisplay(in: bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
    }
    #endif

    // MARK: Commands, reached through the responder chain from the menu

    @objc public func splitRight(_ sender: Any?) { split(.sideBySide) }
    @objc public func splitDown(_ sender: Any?) { split(.stacked) }
    @objc public func closePane(_ sender: Any?) { close(tree.focused) }
    @objc public func focusNextPane(_ sender: Any?) { moveFocus(by: 1) }
    @objc public func focusPreviousPane(_ sender: Any?) { moveFocus(by: -1) }

    // MARK: Panes

    private func split(_ axis: SplitAxis) {
        addPane(tree.splitFocused(axis))
        rebuild()
    }

    private func close(_ id: PaneID) {
        // The last pane ends the window, or the tab it is in.
        guard tree.close(id) else {
            window?.performClose(nil)
            return
        }
        panes[id] = nil
        rebuild()
    }

    private func moveFocus(by step: Int) {
        tree.focusNext(by: step)
        focusCurrent()
    }

    private func addPane(_ id: PaneID) {
        let view = TerminalView(frame: .zero)
        view.takesFocusOnAttach = false
        view.onChildExit = { [weak self] in self?.paneExited(id) }
        view.onFocus = { [weak self] in self?.paneFocused(id) }
        panes[id] = view
    }

    private func paneExited(_ id: PaneID) {
        // The last shell exiting closes the window outright, as it did
        // before there were panes; there is nothing left to confirm.
        if tree.panes == [id] { window?.close() } else { close(id) }
    }

    private func paneFocused(_ id: PaneID) {
        // Re-parenting hands the focus around; only the user's choice counts.
        if !rebuilding { tree.focus(id) }
    }

    private func focusCurrent() {
        window?.makeFirstResponder(panes[tree.focused])
    }

    private func rebuild() {
        rebuilding = true
        panes.values.forEach { $0.removeFromSuperview() }
        subviews.forEach { $0.removeFromSuperview() }
        let root = build(tree.root)
        root.frame = bounds
        root.autoresizingMask = [.width, .height]
        addSubview(root)
        rebuilding = false
        focusCurrent()
    }

    private func build(_ node: PaneNode) -> NSView {
        switch node {
        case .leaf(let id):
            return panes[id] ?? NSView()
        case .split(let axis, let first, let second):
            let split = EvenSplitView()
            split.isVertical = axis == .sideBySide
            split.dividerStyle = .thin
            split.addArrangedSubview(build(first))
            split.addArrangedSubview(build(second))
            return split
        }
    }
}
