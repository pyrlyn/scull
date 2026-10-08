import AppKit
import CScull
import Testing
@testable import ScullKit

@Test func pressesPickTheSelectionKind() {
    let press = SelectionGesture.press
    #expect(press(1, [], false) == .start(kind: TT_SELECT_CELL))
    #expect(press(2, [], false) == .start(kind: TT_SELECT_WORD))
    #expect(press(3, [], true) == .start(kind: TT_SELECT_LINE))
    #expect(press(4, [], false) == .start(kind: TT_SELECT_LINE))
    #expect(press(1, .option, false) == .start(kind: TT_SELECT_BLOCK))
    #expect(press(1, .shift, true) == .extend)
    #expect(press(2, .shift, true) == .extend)
    #expect(press(1, .shift, false) == .start(kind: TT_SELECT_CELL), "nothing to extend")
    #expect(SelectionGesture.forcesSelection(.shift) && !SelectionGesture.forcesSelection(.option))
}

@Test func pointsMapToClampedCells() {
    let cell = { (x: CGFloat, y: CGFloat) in
        SelectionGesture.cell(at: CGPoint(x: x, y: y), cellWidth: 8, cellHeight: 16, cols: 10, rows: 3)
    }
    #expect(cell(0, 0) == (0, 0))
    #expect(cell(17, 33) == (2, 2))
    #expect(cell(-5, -40) == (0, 0))
    #expect(cell(500, 500) == (2, 9))
}

/// A view over a terminal with no child, fed `text`, in a window of its
/// own, with Copy going to a private pasteboard.
@MainActor
private func screen(_ text: String) throws -> (TerminalView, NSWindow, NSPasteboard) {
    let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 100))
    let session = try TerminalSession(cols: 12, rows: 3)
    #expect(session.feed(Array(text.utf8)) == TT_OK)
    session.update()
    session.pasteboard = NSPasteboard(name: NSPasteboard.Name("org.scull.test.\(UUID().uuidString)"))
    view.session = session
    view.setFrameSize(NSSize(width: 12.5 * view.cellWidth, height: 3.5 * view.cellHeight))
    let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView = view
    return (view, window, session.pasteboard)
}

/// A left-button event over the middle of viewport cell `row`, `col`.
@MainActor
private func click(_ view: TerminalView, _ type: NSEvent.EventType, row: Int, col: Int, count: Int = 1,
                   _ flags: NSEvent.ModifierFlags = []) -> NSEvent {
    let point = view.convert(NSPoint(x: (Double(col) + 0.5) * view.cellWidth, y: (Double(row) + 0.5) * view.cellHeight),
                             to: nil)
    return NSEvent.mouseEvent(with: type, location: point, modifierFlags: flags, timestamp: 0,
                              windowNumber: view.window?.windowNumber ?? 0, context: nil, eventNumber: 0,
                              clickCount: count, pressure: 1)!
}

@MainActor
private func drag(_ view: TerminalView, from: (Int, Int), to: (Int, Int), _ flags: NSEvent.ModifierFlags = []) {
    view.mouseDown(with: click(view, .leftMouseDown, row: from.0, col: from.1, flags))
    view.mouseDragged(with: click(view, .leftMouseDragged, row: to.0, col: to.1, flags))
    view.mouseUp(with: click(view, .leftMouseUp, row: to.0, col: to.1, flags))
}

@MainActor
@Test func dragSelectsAndCopyFillsThePasteboard() throws {
    let (view, _, pasteboard) = try screen("hello world\r\nsecond line")
    defer { pasteboard.releaseGlobally() }
    let copyItem = NSMenuItem(title: "Copy", action: #selector(TerminalView.copy(_:)), keyEquivalent: "c")
    #expect(!view.validateMenuItem(copyItem), "nothing to copy yet")
    drag(view, from: (0, 6), to: (1, 2))
    #expect(view.session?.selectionText() == "world\nsec")
    #expect(view.validateMenuItem(copyItem))
    view.copy(nil)
    #expect(pasteboard.string(forType: .string) == "world\nsec")
    // Shift grows it; a click that does not drag selects nothing.
    view.mouseDown(with: click(view, .leftMouseDown, row: 1, col: 5, .shift))
    view.mouseUp(with: click(view, .leftMouseUp, row: 1, col: 5, .shift))
    #expect(view.session?.selectionText() == "world\nsecond")
    view.mouseDown(with: click(view, .leftMouseDown, row: 0, col: 0))
    view.mouseUp(with: click(view, .leftMouseUp, row: 0, col: 0))
    #expect(view.session?.hasSelection == false)
    #expect(!view.validateMenuItem(copyItem))
}

@MainActor
@Test func doubleAndTripleClicksTakeAWordAndALine() throws {
    let (view, _, pasteboard) = try screen("hello world\r\nsecond line")
    defer { pasteboard.releaseGlobally() }
    view.mouseDown(with: click(view, .leftMouseDown, row: 0, col: 8, count: 2))
    view.mouseUp(with: click(view, .leftMouseUp, row: 0, col: 8, count: 2))
    #expect(view.session?.selectionText() == "world")
    view.mouseDown(with: click(view, .leftMouseDown, row: 1, col: 1, count: 3))
    view.mouseUp(with: click(view, .leftMouseUp, row: 1, col: 1, count: 3))
    #expect(view.session?.selectionText() == "second line")
    // Option drags a block: the same columns of both rows.
    drag(view, from: (0, 0), to: (1, 2), .option)
    #expect(view.session?.selectionText() == "hel\nsec")
}

@MainActor
@Test func aTrackingProgramGetsClicksUnlessShiftIsHeld() throws {
    // Mode 1000: the program reports presses and releases.
    let (view, _, pasteboard) = try screen("\u{1b}[?1000hhello world")
    defer { pasteboard.releaseGlobally() }
    drag(view, from: (0, 0), to: (0, 4))
    #expect(view.session?.hasSelection == false, "the program took the drag")
    drag(view, from: (0, 0), to: (0, 4), .shift)
    #expect(view.session?.selectionText() == "hello")
}

@MainActor
@Test func typingClearsTheSelection() throws {
    let (view, _, pasteboard) = try screen("hello")
    defer { pasteboard.releaseGlobally() }
    drag(view, from: (0, 0), to: (0, 2))
    #expect(view.session?.hasSelection == true)
    view.keyDown(with: NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
                                        windowNumber: 0, context: nil, characters: "a", charactersIgnoringModifiers: "a",
                                        isARepeat: false, keyCode: 0)!)
    #expect(view.session?.hasSelection == false)
}
