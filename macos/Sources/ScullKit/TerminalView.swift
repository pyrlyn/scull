// The terminal surface: owns a TerminalSession, draws its frame through
// the Metal renderer on a CAMetalLayer and turns AppKit events into the
// core's input events. Image placements are not drawn yet.

import AppKit
import CoreText
import CScull

public final class TerminalView: NSView {
    private var session: TerminalSession?
    private var failure: String?
    private let palette = Palette()
    private let font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
    private var renderer: MetalRenderer?
    private let cellWidth: CGFloat
    private let cellHeight: CGFloat
    private var grid = (cols: UInt16(80), rows: UInt16(24))
    private var focused = false
    private var keyText: String?
    private var lastMotionCell: (Int, Int)?
    private var scrollRemainder: CGFloat = 0
    #if DEBUG
    private var initialInput = UserDefaults.standard.string(forKey: "ScullInitialInput")
    #endif

    public override init(frame: NSRect) {
        // Advances, not rounded cells, so CoreText's glyphs land on the grid.
        cellWidth = ("M" as NSString).size(withAttributes: [.font: font]).width
        cellHeight = ceil(font.ascender - font.descender + font.leading)
        super.init(frame: frame)
        let env = ["TERM=xterm-256color", "COLORTERM=truecolor"]
        do {
            session = try TerminalSession(cols: grid.cols, rows: grid.rows, env: env,
                                          cwd: NSHomeDirectory()) { [weak self] in self?.wake() }
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

    public override var isFlipped: Bool { true }
    public override var acceptsFirstResponder: Bool { true }

    private func wake() {
        guard let session else { return }
        if session.drainEvents(bell: { NSSound.beep() }) {
            window?.close()
            return
        }
        if session.update() { needsDisplay = true }
        #if DEBUG
        // Lets a scripted launch type a command once the shell is there.
        if let input = initialInput {
            initialInput = nil
            _ = session.text(input)
            send(key: UInt32(TT_KEY_ESCAPE) + 1, action: UInt8(TT_KEY_PRESS), mods: 0, text: "")
            if let path = UserDefaults.standard.string(forKey: "ScullSnapshot") {
                // A shell with heavy startup needs longer than the default.
                let delay = max(2, UserDefaults.standard.double(forKey: "ScullSnapshotDelay"))
                DispatchQueue.main.asyncAfter(deadline: .now() + delay) { [weak self] in self?.snapshot(to: path) }
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
    #endif

    // MARK: Size

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        NotificationCenter.default.removeObserver(self)
        guard let window else { return }
        window.makeFirstResponder(self)
        focused = window.isKeyWindow
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
        renderer?.draw(session, size: bounds.size, scale: window?.backingScaleFactor ?? 1, focused: focused)
    }

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
        focused = note.name == NSWindow.didBecomeKeyNotification
        _ = session?.focus(focused)
        needsDisplay = true
    }

    public override func keyDown(with event: NSEvent) {
        let flags = event.modifierFlags
        guard !flags.contains(.command) else { return super.keyDown(with: event) }
        var text = ""
        if flags.contains(.control) {
            // Control's own characters are the core's to produce; the
            // event's text is what the key types without it.
            text = event.charactersIgnoringModifiers ?? ""
        } else {
            // Collects what the input system types, dead keys included.
            keyText = ""
            interpretKeyEvents([event])
            text = keyText ?? ""
            keyText = nil
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

    public override func insertText(_ insertString: Any) {
        let string = (insertString as? NSAttributedString)?.string ?? (insertString as? String) ?? ""
        if keyText != nil {
            keyText? += string
        } else if session?.text(string) == TT_FULL {
            NSSound.beep()
        }
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
        let col = min(max(0, Int(p.x / cellWidth)), Int(grid.cols) - 1)
        let row = min(max(0, Int(p.y / cellHeight)), Int(grid.rows) - 1)
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

    public override func mouseDown(with e: NSEvent) { mouse(e, action: TT_MOUSE_PRESS, button: TT_MOUSE_LEFT) }
    public override func mouseUp(with e: NSEvent) { mouse(e, action: TT_MOUSE_RELEASE, button: TT_MOUSE_LEFT) }
    public override func mouseDragged(with e: NSEvent) { mouse(e, action: TT_MOUSE_MOTION, button: TT_MOUSE_LEFT) }
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
