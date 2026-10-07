// The terminal surface: owns a TerminalSession, draws its frame with
// CoreText and turns AppKit events into the core's input events. A plain
// CoreText pass is the first light; the Metal renderer with a glyph
// atlas replaces `draw` later. Image placements are not drawn yet.

import AppKit
import CoreText
import CScull

public final class TerminalView: NSView {
    var session: TerminalSession?
    private var failure: String?
    private let config = ScullConfig.shared
    private(set) var palette: Palette
    private var fonts: FontSet
    /// Points the font-larger and font-smaller actions added; a changed
    /// font in the file starts from its own size again.
    private var zoom = 0.0
    private var appliedFont: (family: String, size: Double)
    private var configObserver: (any NSObjectProtocol)?
    var font: NSFont { fonts.regular }
    private var bold: NSFont { fonts.bold }
    private var italic: NSFont { fonts.italic }
    private var boldItalic: NSFont { fonts.boldItalic }
    var cellWidth: CGFloat { fonts.cellWidth }
    var cellHeight: CGFloat { fonts.cellHeight }
    private var grid = (cols: UInt16(80), rows: UInt16(24))
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
    #if DEBUG
    private var initialInput = UserDefaults.standard.string(forKey: "ScullInitialInput")
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
        if session.drainEvents(bell: { NSSound.beep() }) {
            if let onChildExit { onChildExit() } else { window?.close() }
            return
        }
        if session.update() { needsDisplay = true; screenChanged() }
        #if DEBUG
        // Lets a scripted launch type a command once the shell is there.
        if let input = initialInput {
            initialInput = nil
            _ = session.text(input)
            composeDebugPreedit()
            send(key: UInt32(TT_KEY_ESCAPE) + 1, action: UInt8(TT_KEY_PRESS), mods: 0, text: "")
            if let path = UserDefaults.standard.string(forKey: "ScullSnapshot") {
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in self?.snapshot(to: path) }
            }
        }
        #endif
    }

    #if DEBUG
    // The view draws itself into a PNG, so a scripted check needs no
    // screen-recording permission.
    private func snapshot(to path: String) {
        guard let rep = bitmapImageRepForCachingDisplay(in: bounds) else { return }
        cacheDisplay(in: bounds, to: rep)
        try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
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

    public override func draw(_ dirtyRect: NSRect) {
        guard let ctx = NSGraphicsContext.current?.cgContext else { return }
        ctx.setFillColor(Palette.cgColor(palette.background))
        ctx.fill(bounds)
        // A crashed core terminal shows the notice in place of its stale grid.
        if let notice = failure ?? (session?.isPoisoned == true ? "This terminal crashed. Close the pane to dismiss it." : nil) {
            NSAttributedString(string: notice, attributes: [.font: font, .foregroundColor: NSColor.white])
                .draw(at: NSPoint(x: cellWidth, y: cellHeight))
            return
        }
        guard let session else { return }
        let view = session.view
        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        for row in 0..<Int(view.rows) {
            drawBackgrounds(session, row: row, in: ctx)
            for (run, text) in session.runs(row: row) {
                drawRun(text, style: session.style(run.style), col: Int(run.col), cols: Int(run.cols),
                        width: Int(run.width), row: row, in: ctx)
            }
        }
        drawPreedit(in: ctx)
        drawCursor(session, in: ctx)
    }

    private func cellRect(row: Int, col: Int, cols: Int = 1) -> CGRect {
        CGRect(x: CGFloat(col) * cellWidth, y: CGFloat(row) * cellHeight,
               width: CGFloat(cols) * cellWidth, height: cellHeight)
    }

    private func drawBackgrounds(_ session: TerminalSession, row: Int, in ctx: CGContext) {
        for col in 0..<Int(session.view.cols) {
            guard let cell = session.cell(row: row, col: col) else { return }
            let bg = palette.colors(of: session.style(cell.style)).bg
            guard bg != palette.background else { continue }
            ctx.setFillColor(Palette.cgColor(bg))
            ctx.fill(cellRect(row: row, col: col).integral)
        }
    }

    private func drawRun(_ text: String, style: tt_style, col: Int, cols: Int, width: Int,
                         row: Int, in ctx: CGContext, color: UInt32? = nil) {
        let attrs = style.attrs
        guard attrs & UInt16(TT_ATTR_HIDDEN) == 0 else { return }
        let fg = color ?? palette.colors(of: style).fg
        let dim = attrs & UInt16(TT_ATTR_DIM) != 0
        let cg = Palette.cgColor(fg, alpha: dim ? 0.5 : 1)
        let isBold = attrs & UInt16(TT_ATTR_BOLD) != 0
        let isItalic = attrs & UInt16(TT_ATTR_ITALIC) != 0
        let face = isBold ? (isItalic ? boldItalic : bold) : (isItalic ? italic : font)
        let baseline = CGFloat(row) * cellHeight + font.ascender
        // A wide character's fallback glyph rarely advances exactly two
        // cells, so each one is placed on its own cell instead.
        let pieces = width == 2 ? text.map { String($0) } : [text]
        for (i, piece) in pieces.enumerated() {
            let string = NSAttributedString(string: piece, attributes: [
                NSAttributedString.Key(kCTFontAttributeName as String): face,
                NSAttributedString.Key(kCTForegroundColorAttributeName as String): cg,
            ])
            ctx.textPosition = CGPoint(x: CGFloat(col + i * width) * cellWidth, y: baseline)
            CTLineDraw(CTLineCreateWithAttributedString(string), ctx)
        }
        // Lines are drawn by hand: CTLineDraw ignores strikethrough, and all
        // underline kinds share one line until the renderer draws them.
        let span = cellRect(row: row, col: col, cols: cols)
        var lines: [(y: CGFloat, color: CGColor)] = []
        if style.underline != UInt8(TT_UNDERLINE_NONE) {
            let color = style.underline_color == UInt32(TT_COLOR_DEFAULT) ? cg
                : Palette.cgColor(palette.rgb(style.underline_color, fallback: fg))
            lines.append((baseline - font.underlinePosition, color))
        }
        if attrs & UInt16(TT_ATTR_STRIKE) != 0 { lines.append((baseline - font.xHeight / 2, cg)) }
        if attrs & UInt16(TT_ATTR_OVERLINE) != 0 { lines.append((span.minY, cg)) }
        for (y, color) in lines {
            ctx.setFillColor(color)
            ctx.fill(CGRect(x: span.minX, y: y, width: span.width, height: max(1, font.underlineThickness)))
        }
    }

    private func drawCursor(_ session: TerminalSession, in ctx: CGContext) {
        let cursor = session.view.cursor
        let (row, col) = (Int(cursor.row), Int(cursor.col))
        guard cursor.visible != 0, let cell = session.cell(row: row, col: col) else { return }
        let rect = cellRect(row: row, col: col, cols: max(1, Int(cell.width)))
        let color = Palette.cgColor(palette.cursor)
        guard focused else {
            ctx.setStrokeColor(color)
            ctx.stroke(rect.insetBy(dx: 0.5, dy: 0.5), width: 1)
            return
        }
        ctx.setFillColor(color)
        ctx.fill(rect)
        guard cell.codepoint != 0, let scalar = Unicode.Scalar(cell.codepoint) else { return }
        drawRun(String(scalar), style: session.style(cell.style), col: col, cols: Int(cell.width),
                width: Int(cell.width), row: row, in: ctx, color: palette.background)
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
        return true
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
