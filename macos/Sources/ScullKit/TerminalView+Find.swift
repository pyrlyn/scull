// Find in this pane's scrollback: the menu's Find commands arrive here
// through the responder chain, also from the bar's own search field,
// which sits inside the view.

import AppKit
import CScull

extension TerminalView {
    @objc public func showFind(_ sender: Any?) {
        let bar = findBar ?? makeFindBar()
        findChanged(find.open())
        window?.makeFirstResponder(bar.field)
        bar.field.selectText(nil)
    }

    @objc public func findNext(_ sender: Any?) { findStep(forward: true) }
    @objc public func findPrevious(_ sender: Any?) { findStep(forward: false) }

    private func findStep(forward: Bool) {
        findSettle?.invalidate()
        apply(find.step(forward: forward))
    }

    func findEdited(_ text: String) { findChanged(find.edit(text)) }

    /// Runs a changed pattern's effects at once, so visible matches light
    /// up while typing, and the history-wide step and count after a pause.
    private func findChanged(_ effects: [FindEffect]) {
        apply(effects)
        findSettle?.invalidate()
        guard find.unsettled else { return }
        findSettle = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.apply(self.find.settle())
            }
        }
    }

    func closeFind() {
        findSettle?.invalidate()
        apply(find.close())
        findBar?.removeFromSuperview()
        findBar = nil
        window?.makeFirstResponder(self)
    }

    func apply(_ effects: [FindEffect]) {
        guard let session else { return }
        for effect in effects {
            switch effect {
            case let .search(pattern, ignoreCase): _ = session.search(pattern, ignoreCase: ignoreCase)
            case let .step(forward): _ = session.searchStep(forward: forward)
            case .count: find.count = session.searchCount()
            }
        }
        findBar?.show(find)
        if session.update() { needsDisplay = true }
    }

    private func makeFindBar() -> FindBar {
        let bar = FindBar()
        bar.pattern = find.pattern
        bar.onEdit = { [weak self] in self?.findEdited($0) }
        bar.onStep = { [weak self] in self?.findStep(forward: $0) }
        bar.onToggleCase = { [weak self] in
            guard let self else { return }
            findChanged(find.toggleCase())
        }
        bar.onClose = { [weak self] in self?.closeFind() }
        bar.translatesAutoresizingMaskIntoConstraints = false
        addSubview(bar)
        NSLayoutConstraint.activate([
            bar.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            bar.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
        ])
        findBar = bar
        return bar
    }
}
