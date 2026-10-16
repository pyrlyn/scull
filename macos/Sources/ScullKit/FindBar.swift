// Find in the scrollback: a state machine that says what to ask of the
// core, and the floating bar that drives it. TerminalView+Find joins them.

import AppKit
import CScull

/// A core call the find bar's state asks for.
enum FindEffect: Equatable {
    /// Replace the search; an empty pattern ends it.
    case search(String, ignoreCase: Bool)
    /// Step to the next match (forward) or the previous one.
    case step(forward: Bool)
    /// Count the matches.
    case count
}

struct FindState: Equatable {
    private(set) var isOpen = false
    /// Kept across a close, so the bar reopens on the last search.
    private(set) var pattern = ""
    private(set) var ignoreCase = true
    /// Counted once the pattern settles; nil until then.
    var count: Int?
    /// A step and a count both rescan the whole history, so typing waits
    /// for the pattern to settle before either runs.
    private(set) var unsettled = false

    var canStep: Bool { isOpen && !pattern.isEmpty }

    mutating func open() -> [FindEffect] {
        guard !isOpen else { return [] }
        isOpen = true
        return pattern.isEmpty ? [] : edit(pattern)
    }

    mutating func edit(_ text: String) -> [FindEffect] {
        guard isOpen else { return [] }
        (pattern, count, unsettled) = (text, nil, !text.isEmpty)
        return [.search(text, ignoreCase: ignoreCase)]
    }

    mutating func toggleCase() -> [FindEffect] {
        ignoreCase.toggle()
        return edit(pattern)
    }

    /// The pattern stopped changing: show the first match and count.
    mutating func settle() -> [FindEffect] {
        guard unsettled, canStep else { return [] }
        unsettled = false
        return [.step(forward: true), .count]
    }

    mutating func step(forward: Bool) -> [FindEffect] {
        guard canStep else { return [] }
        // Stepping now stands in for the settle's own first step.
        let counts = count == nil
        unsettled = false
        return counts ? [.step(forward: forward), .count] : [.step(forward: forward)]
    }

    mutating func close() -> [FindEffect] {
        guard isOpen else { return [] }
        (isOpen, count, unsettled) = (false, nil, false)
        return [.search("", ignoreCase: ignoreCase)]
    }

    var label: String {
        switch count {
        case nil: ""
        case 0: "No matches"
        case 1: "1 match"
        case let n? where n >= Int(TT_MAX_SEARCH_MATCHES): "\(n)+ matches"
        case let n?: "\(n) matches"
        }
    }
}

/// The bar over the terminal's top-right corner. It reports what the user
/// did; the terminal view owns the state.
final class FindBar: NSVisualEffectView, NSSearchFieldDelegate {
    var onEdit: (String) -> Void = { _ in }
    var onStep: (_ forward: Bool) -> Void = { _ in }
    var onToggleCase: () -> Void = {}
    var onClose: () -> Void = {}

    let field = NSSearchField()
    private let status = NSTextField(labelWithString: "")
    private let caseButton = NSButton(title: "Aa", target: nil, action: nil)

    init() {
        super.init(frame: .zero)
        (material, blendingMode, state) = (.popover, .withinWindow, .active)
        wantsLayer = true
        layer?.cornerRadius = 8
        field.placeholderString = "Find"
        field.delegate = self
        field.widthAnchor.constraint(equalToConstant: 180).isActive = true
        status.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        status.textColor = .secondaryLabelColor
        caseButton.setButtonType(.pushOnPushOff)
        caseButton.bezelStyle = .accessoryBarAction
        caseButton.toolTip = "Match Case"
        caseButton.target = self
        caseButton.action = #selector(toggleCase)
        let arrows = NSSegmentedControl(images: [Self.symbol("chevron.up", "Previous"), Self.symbol("chevron.down", "Next")],
                                        trackingMode: .momentary, target: self, action: #selector(arrow))
        arrows.segmentStyle = .separated
        let done = NSButton(image: Self.symbol("xmark.circle.fill", "Close"), target: self, action: #selector(close))
        done.isBordered = false
        let stack = NSStackView(views: [field, caseButton, status, arrows, done])
        (stack.spacing, stack.edgeInsets) = (6, NSEdgeInsets(top: 6, left: 8, bottom: 6, right: 8))
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor), stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.topAnchor.constraint(equalTo: topAnchor), stack.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    private static func symbol(_ name: String, _ label: String) -> NSImage {
        NSImage(systemSymbolName: name, accessibilityDescription: label) ?? NSImage()
    }

    var pattern: String {
        get { field.stringValue }
        set { field.stringValue = newValue }
    }

    func show(_ state: FindState) {
        status.stringValue = state.label
        caseButton.state = state.ignoreCase ? .off : .on
    }

    @objc private func toggleCase() { onToggleCase() }
    @objc private func arrow(_ sender: NSSegmentedControl) { onStep(sender.selectedSegment == 1) }
    @objc private func close() { onClose() }

    func controlTextDidChange(_ note: Notification) { onEdit(field.stringValue) }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.insertNewline(_:)):
            onStep(!(NSApp.currentEvent?.modifierFlags.contains(.shift) ?? false))
        // Escape ends the search outright; the field would only clear itself.
        case #selector(NSResponder.cancelOperation(_:)): onClose()
        default: return false
        }
        return true
    }
}
