// The terminal surface: owns a TerminalSession, draws its frame through
// the Metal renderer on a CAMetalLayer and turns AppKit events into the
// core's input events.

import AppKit
import CoreText
import CScull

public final class TerminalView: NSView {
    var session: TerminalSession?
    private var failure: String?
    private let config = ScullConfig.shared
    private(set) var palette: Palette { didSet { renderer?.palette = palette } }
    private var fonts: FontSet {
        didSet { renderer?.setFont(fonts.regular, cellWidth: fonts.cellWidth, cellHeight: fonts.cellHeight) }
    }
    private var renderer: MetalRenderer?
    /// Points the font-larger and font-smaller actions added; a changed
    /// font in the file starts from its own size again.
    private var zoom = 0.0
    private var appliedFont: (family: String, size: Double)
    private var configObserver: (any NSObjectProtocol)?
    var font: NSFont { fonts.regular }
    var cellWidth: CGFloat { fonts.cellWidth }
    var cellHeight: CGFloat { fonts.cellHeight }
    private(set) var grid = (cols: UInt16(80), rows: UInt16(24))
    private var focused = false
    /// Set by a pane host: the child's exit closes the pane, not the window.
    public var onChildExit: (() -> Void)?
    /// Runs when this view becomes the first responder, so a host can track
    /// which pane has the keyboard.
    public var onFocus: (() -> Void)?
    /// A host with several panes picks the focused one itself.
    public var takesFocusOnAttach = true
    var textInput = TextInput()
    private var lastMotionCell: (Int, Int)?
    private var scrollRemainder: CGFloat = 0
    /// The left-button gesture the host owns; nil while the program has it.
    var selectionDrag: SelectionDrag?
    var findBar: FindBar?
    var find = FindState()
    var findSettle: Timer?
    #if DEBUG
    private var initialInput = UserDefaults.standard.string(forKey: "ScullInitialInput")
    private var probe: LatencyProbe?
    #endif

    public override init(frame: NSRect) {
        let settings = config.settings
        palette = Palette(settings)
        fonts = FontSet(family: settings.fontFamily, size: settings.fontSize)
        appliedFont = (settings.fontFamily, settings.fontSize)
        super.init(frame: frame)
        configObserver = NotificationCenter.default.addObserver(
            forName: ScullConfig.didChange, object: config, queue: .main
        ) { [weak self] _ in MainActor.assumeIsolated { self?.applyConfig() } }
        let env = ["TERM=xterm-256color", "COLORTERM=truecolor"]
        do {
            session = try TerminalSession(cols: grid.cols, rows: grid.rows, env: env,
                                          cwd: NSHomeDirectory(), scrollback: settings.scrollback) { [weak self] in self?.wake() }
        } catch {
            failure = "Could not start the shell: \(error)"
        }
        guard session != nil else { return }
        renderer = MetalRenderer(font: font, cellWidth: cellWidth, cellHeight: cellHeight, palette: palette)
        if renderer == nil { failure = "Metal is not available" }
        wantsLayer = true
        layerContentsRedrawPolicy = .duringViewResize
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    isolated deinit {
        configObserver.map(NotificationCenter.default.removeObserver)
    }

    public override var isFlipped: Bool { true }
    public override var acceptsFirstResponder: Bool { true }

    private func wake() {
        guard let session else { return }
        let title = session.title
        if session.drainEvents(bell: { NSSound.beep() }) {
            if let onChildExit { onChildExit() } else { window?.close() }
            return
        }
        if session.title != title { showTitle(window?.firstResponder === self) }
        if session.update() {
            needsDisplay = true
            screenChanged()
            #if DEBUG
            if let renderer, let probe {
                probe.frameUpdated(renderer)
                // AppKit skips drawing a covered window; the probe still
                // wants the render time.
                if window?.occlusionState.contains(.visible) == false { updateLayer() }
            }
            #endif
        }
        #if DEBUG
        // Lets a scripted launch type a command once the shell is there.
        if let input = initialInput {
            initialInput = nil
            _ = session.text(input)
            composeDebugPreedit()
            send(key: UInt32(TT_KEY_ESCAPE) + 1, action: UInt8(TT_KEY_PRESS), mods: 0, text: "")
            if let path = UserDefaults.standard.string(forKey: "ScullSnapshot") {
                // A shell with heavy startup needs longer than the default.
                let delay = max(2, UserDefaults.standard.double(forKey: "ScullSnapshotDelay"))
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in
                    self?.markForSnapshot()
                    self?.snapshot(to: path)
                }
            }
            if let path = UserDefaults.standard.string(forKey: "ScullLatencyProbe") {
                probe = LatencyProbe(path: path) { [weak self] in
                    self?.send(key: 0x78, action: UInt8(TT_KEY_PRESS), mods: 0, text: "x")
                }
                let delay = max(2, UserDefaults.standard.double(forKey: "ScullSnapshotDelay"))
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in self?.probe?.next() }
            }
        }
        #endif
    }

    #if DEBUG
    // The view draws itself into a PNG, so a scripted check needs no
    // screen-recording permission.
    private func snapshot(to path: String) {
        guard let session, let image = renderer?.snapshot(session, size: bounds.size,
                                                           scale: window?.backingScaleFactor ?? 1, focused: focused)
        else { return }
        try? NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])?
            .write(to: URL(fileURLWithPath: path))
    }

    /// `-ScullFind <pattern>` and `-ScullSelect <row,col,row,col>` put a
    /// highlighted match and a selection into the snapshot.
    private func markForSnapshot() {
        guard let session else { return }
        if let pattern = UserDefaults.standard.string(forKey: "ScullFind") {
            showFind(nil)
            findBar?.pattern = pattern
            findEdited(pattern)
            apply(find.settle())
        }
        let ends = UserDefaults.standard.string(forKey: "ScullSelect")?.split(separator: ",").compactMap { Int($0) }
        if let ends, ends.count == 4 {
            _ = session.select(TT_SELECT_CELL, row: ends[0], col: ends[1])
            _ = session.extendSelection(row: ends[2], col: ends[3])
        }
        session.update()
    }
    #endif

    // MARK: Configuration

    private func applyConfig() {
        let settings = config.settings
        palette = Palette(settings)
        if (settings.fontFamily, settings.fontSize) != appliedFont {
            appliedFont = (settings.fontFamily, settings.fontSize)
            zoom = 0
        }
        applyFont()
        #if DEBUG
        // Shows a scripted check that an edit of the file reached the view.
        if initialInput == nil, let path = UserDefaults.standard.string(forKey: "ScullSnapshotOnConfig") {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in self?.snapshot(to: path) }
        }
        #endif
    }

    private func applyFont() {
        let size = min(max(appliedFont.size + zoom, 4), 200)
        fonts = FontSet(family: appliedFont.family, size: size)
        resizeGrid(force: true)
        needsDisplay = true
    }

    /// Runs the action bound to the key and modifiers of `event`, if any.
    private func runBinding(_ event: NSEvent) -> Bool {
        let shortcutMods = UInt8(TT_MOD_SHIFT | TT_MOD_ALT | TT_MOD_CTRL | TT_MOD_SUPER)
        let unshifted = event.characters(byApplyingModifiers: [])?.unicodeScalars.first?.value
        guard let key = KeyMap.key(forKeyCode: event.keyCode) ?? unshifted else { return false }
        let mods = KeyMap.mods(event.modifierFlags) & shortcutMods
        guard let bind = config.settings.keybinds.first(where: { $0.key == key && $0.mods == mods }) else {
            return false
        }
        switch bind.action {
        case .paste: paste(nil)
        case .fontLarger: zoom += 1; applyFont()
        case .fontSmaller: zoom -= 1; applyFont()
        case .fontReset: zoom = 0; applyFont()
        case .scrollPageUp: scroll(Int32(grid.rows))
        case .scrollPageDown: scroll(-Int32(grid.rows))
        case .scrollToTop: scroll(Int32.max)
        case .scrollToBottom: scroll(-Int32.max)
        }
        return true
    }

    private func scroll(_ rows: Int32) {
        guard let session else { return }
        _ = session.scrollDisplay(rows)
        if session.update() { needsDisplay = true }
    }

    // MARK: Size

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        NotificationCenter.default.removeObserver(self)
        guard let window else { return }
        if takesFocusOnAttach { window.makeFirstResponder(self) }
        focused = window.isKeyWindow && window.firstResponder === self
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            NotificationCenter.default.addObserver(self, selector: #selector(keyChanged),
                                                   name: name, object: window)
        }
        resizeGrid()
    }

    public override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        if !inLiveResize { resizeGrid() }
    }

    public override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        resizeGrid(force: true)
    }

    // Holding output back during a drag keeps the history from being
    // rewrapped at every intermediate width.
    public override func viewWillStartLiveResize() {
        super.viewWillStartLiveResize()
        _ = session?.resizeBegin()
    }

    public override func viewDidEndLiveResize() {
        super.viewDidEndLiveResize()
        resizeGrid(force: true)
    }

    private func resizeGrid(force: Bool = false) {
        guard let session, window != nil else { return }
        let cols = UInt16(clamping: max(1, Int(bounds.width / cellWidth)))
        let rows = UInt16(clamping: max(1, Int(bounds.height / cellHeight)))
        guard force || (cols, rows) != grid else { return }
        grid = (cols, rows)
        let scale = window?.backingScaleFactor ?? 1
        let px = { (cells: UInt16, size: CGFloat) in UInt16(clamping: Int(CGFloat(cells) * size * scale)) }
        _ = session.resize(cols: cols, rows: rows, widthPx: px(cols, cellWidth), heightPx: px(rows, cellHeight))
        if session.update() { needsDisplay = true }
    }

    // MARK: Drawing

    public override func makeBackingLayer() -> CALayer { renderer?.layer ?? super.makeBackingLayer() }

    public override var wantsUpdateLayer: Bool { renderer != nil }

    public override func updateLayer() {
        guard let session else { return }
        guard !session.isPoisoned else { return showCrashNotice() }
        renderer?.draw(session, size: bounds.size, scale: window?.backingScaleFactor ?? 1, focused: focused)
    }

    // A crashed core terminal shows the notice in place of its stale grid.
    // The Metal layer has no text of its own, so the notice is a label on
    // top.
    private func showCrashNotice() {
        guard !subviews.contains(where: { $0.identifier == Self.crashNotice }) else { return }
        let label = NSTextField(labelWithString: "This terminal crashed. Close the pane to dismiss it.")
        (label.identifier, label.font, label.textColor) = (Self.crashNotice, font, .white)
        (label.drawsBackground, label.backgroundColor) = (true, NSColor(cgColor: Palette.cgColor(palette.background)))
        label.frame = bounds
        label.autoresizingMask = [.width, .height]
        addSubview(label)
    }

    private static let crashNotice = NSUserInterfaceItemIdentifier("crashNotice")

    // Only a terminal that failed to start draws here.
    public override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.setFillColor(Palette.cgColor(palette.background))
        ctx.fill(bounds)
        NSAttributedString(string: failure ?? "", attributes: [.font: font, .foregroundColor: NSColor.white])
            .draw(at: NSPoint(x: cellWidth, y: cellHeight))
    }

    // MARK: Keyboard

    @objc private func keyChanged(_ note: Notification) {
        setFocused(note.name == NSWindow.didBecomeKeyNotification && window?.firstResponder === self)
    }

    // Focus is per pane: the key window alone does not say which of its
    // panes has the keyboard.
    public override func becomeFirstResponder() -> Bool {
        setFocused(window?.isKeyWindow ?? false)
        onFocus?()
        // The window is not told its new first responder until this returns.
        showTitle(true)
        return true
    }

    /// The program's title names the window while this pane has the keyboard.
    private func showTitle(_ hasKeyboard: Bool) {
        guard hasKeyboard, let title = session?.title else { return }
        window?.title = title
    }

    public override func resignFirstResponder() -> Bool {
        setFocused(false)
        return true
    }

    private func setFocused(_ value: Bool) {
        guard value != focused else { return }
        focused = value
        _ = session?.focus(value)
        needsDisplay = true
    }

    public override func performKeyEquivalent(with event: NSEvent) -> Bool {
        runBinding(event) || super.performKeyEquivalent(with: event)
    }

    public override func keyDown(with event: NSEvent) {
        if runBinding(event) { return }
        let flags = event.modifierFlags
        guard !flags.contains(.command) else { return super.keyDown(with: event) }
        // Typing replaces what was selected, as in a text view.
        _ = session?.clearSelection()
        var text = ""
        if flags.contains(.control) && !textInput.hasMarkedText {
            // Control's own characters are the core's to produce; the
            // event's text is what the key types without it.
            text = event.charactersIgnoringModifiers ?? ""
        } else {
            // An input method may take the key or compose text from it.
            guard let typed = interpret(event) else { return }
            text = typed
        }
        text = String(text.unicodeScalars.filter { $0.value >= 0x20 && $0.value != 0x7F && !(0xF700...0xF8FF).contains($0.value) })
        sendKey(event, action: event.isARepeat ? TT_KEY_REPEAT : TT_KEY_PRESS, text: text)
    }

    public override func keyUp(with event: NSEvent) {
        sendKey(event, action: TT_KEY_RELEASE, text: "")
    }

    private func sendKey(_ event: NSEvent, action: Int32, text: String) {
        let unshifted = event.characters(byApplyingModifiers: [])?.unicodeScalars.first?.value ?? 0
        guard let key = KeyMap.key(forKeyCode: event.keyCode) ?? (unshifted == 0 ? nil : unshifted) else { return }
        let mods = KeyMap.mods(event.modifierFlags)
        // Option typed the text itself (å), so the core must not add Alt.
        let optionTyped = event.modifierFlags.contains(.option) && !text.isEmpty
            && text != event.charactersIgnoringModifiers
        send(key: key, action: UInt8(action), mods: mods, text: text,
             unshifted: unshifted == key ? 0 : unshifted, consumed: optionTyped ? UInt8(TT_MOD_ALT) : 0)
    }

    private func send(key: UInt32, action: UInt8, mods: UInt8, text: String,
                      unshifted: UInt32 = 0, consumed: UInt8 = 0) {
        guard let session else { return }
        var event = tt_key_event()
        (event.key, event.unshifted, event.action) = (key, unshifted, action)
        (event.mods, event.consumed_mods) = (mods, consumed)
        if session.key(event, text: text) == TT_FULL { NSSound.beep() }
        // Typing returns the view to the screen; show it now.
        if session.update() { needsDisplay = true }
    }

    // Keys like Return and the arrows reach the core as keys, not as the
    // editing commands AppKit maps them to.
    public override func doCommand(by selector: Selector) {}

    @objc public func paste(_ sender: Any?) {
        guard let text = NSPasteboard.general.string(forType: .string), let session else { return }
        if session.paste(text) == TT_FULL { NSSound.beep() }
    }

    // MARK: Mouse

    public override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseMoved, .activeInKeyWindow, .inVisibleRect],
                                       owner: self))
    }

    /// Sends a mouse event at the pointer; true when the program took it.
    @discardableResult
    private func mouse(_ event: NSEvent, action: Int32, button: Int32) -> Bool {
        guard let session else { return false }
        let p = convert(event.locationInWindow, from: nil)
        let (row, col) = cell(at: p)
        if action == TT_MOUSE_MOTION {
            guard lastMotionCell.map({ $0 != (col, row) }) ?? true else { return false }
        }
        lastMotionCell = (col, row)
        let scale = window?.backingScaleFactor ?? 1
        var e = tt_mouse_event()
        (e.action, e.button, e.mods) = (UInt8(action), UInt8(button), KeyMap.mods(event.modifierFlags))
        (e.col, e.row) = (UInt32(col), UInt32(row))
        (e.x_px, e.y_px) = (UInt32(max(0, p.x * scale)), UInt32(max(0, p.y * scale)))
        return session.mouse(e)
    }

    private func otherButton(_ event: NSEvent) -> Int32 {
        // AppKit numbers buttons from 0; X11 skips the wheel's 4 to 7.
        switch event.buttonNumber {
        case 2: TT_MOUSE_MIDDLE
        case let n: Int32(n) + 5
        }
    }

    func cell(at point: CGPoint) -> (row: Int, col: Int) {
        SelectionGesture.cell(at: point, cellWidth: cellWidth, cellHeight: cellHeight,
                              cols: Int(grid.cols), rows: Int(grid.rows))
    }

    public override func mouseDown(with e: NSEvent) {
        // The program sees the press first, as before selection existed.
        if !SelectionGesture.forcesSelection(e.modifierFlags), mouse(e, action: TT_MOUSE_PRESS, button: TT_MOUSE_LEFT) {
            selectionDrag = nil
            return
        }
        selectionPress(e)
    }

    public override func mouseUp(with e: NSEvent) {
        if selectionDrag != nil { return selectionRelease() }
        mouse(e, action: TT_MOUSE_RELEASE, button: TT_MOUSE_LEFT)
    }

    public override func mouseDragged(with e: NSEvent) {
        if selectionDrag != nil { return selectionDragged(e) }
        mouse(e, action: TT_MOUSE_MOTION, button: TT_MOUSE_LEFT)
    }
    public override func mouseMoved(with e: NSEvent) { mouse(e, action: TT_MOUSE_MOTION, button: TT_MOUSE_NONE) }
    public override func rightMouseDown(with e: NSEvent) { mouse(e, action: TT_MOUSE_PRESS, button: TT_MOUSE_RIGHT) }
    public override func rightMouseUp(with e: NSEvent) { mouse(e, action: TT_MOUSE_RELEASE, button: TT_MOUSE_RIGHT) }
    public override func rightMouseDragged(with e: NSEvent) { mouse(e, action: TT_MOUSE_MOTION, button: TT_MOUSE_RIGHT) }
    public override func otherMouseDown(with e: NSEvent) { mouse(e, action: TT_MOUSE_PRESS, button: otherButton(e)) }
    public override func otherMouseUp(with e: NSEvent) { mouse(e, action: TT_MOUSE_RELEASE, button: otherButton(e)) }
    public override func otherMouseDragged(with e: NSEvent) { mouse(e, action: TT_MOUSE_MOTION, button: otherButton(e)) }

    public override func scrollWheel(with event: NSEvent) {
        guard let session else { return }
        // Trackpads report points; whole rows become wheel steps.
        let delta = event.hasPreciseScrollingDeltas ? event.scrollingDeltaY / cellHeight : event.scrollingDeltaY
        scrollRemainder += delta
        let steps = Int(scrollRemainder.rounded(.towardZero))
        guard steps != 0 else { return }
        scrollRemainder -= CGFloat(steps)
        let button = steps > 0 ? TT_MOUSE_WHEEL_UP : TT_MOUSE_WHEEL_DOWN
        if mouse(event, action: TT_MOUSE_PRESS, button: button) {
            for _ in 1..<abs(steps) { mouse(event, action: TT_MOUSE_PRESS, button: button) }
        } else {
            // History scrolling sends no wakeup, so the view refreshes here.
            _ = session.scrollDisplay(Int32(clamping: steps))
            if session.update() { needsDisplay = true }
        }
    }
}
