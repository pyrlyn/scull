import AppKit
import CScull
import Testing
@testable import ScullKit

@MainActor
@Test func aLinkedCellAnswersItsURIAndHoveringFlagsItsCells() throws {
    let session = try TerminalSession(cols: 8, rows: 2)
    // A URI longer than any first guess, so the copy has to measure it.
    let uri = "https://example.com/" + String(repeating: "a", count: 300)
    _ = session.feed(Array("x\u{1b}]8;;\(uri)\u{1b}\\ab\u{1b}]8;;\u{1b}\\".utf8))
    #expect(session.linkAt(row: 0, col: 0) == nil)
    let id = try #require(session.linkAt(row: 0, col: 1))
    #expect(session.linkAt(row: 0, col: 2) == id)
    #expect(session.linkURI(id) == uri)
    #expect(session.linkURI(id + 1) == nil)
    #expect(session.linkAt(row: -1, col: 0) == nil)

    session.hoverLink(id)
    session.update()
    let flags = (0..<4).map { session.cell(row: 0, col: $0)!.flags & UInt8(TT_CELL_LINK) }
    #expect(flags == [0, UInt8(TT_CELL_LINK), UInt8(TT_CELL_LINK), 0])
    session.hoverLink(nil)
    session.update()
    #expect(session.cell(row: 0, col: 1)!.flags & UInt8(TT_CELL_LINK) == 0)
}

@Test func theCoreDecidesWhichLinksOpen() {
    let of = { (uri: String) in LinkTarget.of(uri, machine: "mymac.local") }
    #expect(of("https://example.com/a?b=1") == .web(URL(string: "https://example.com/a?b=1")!))
    #expect(of("HTTP://example.com") != nil)
    #expect(of("mailto:someone@example.com") == .mail(URL(string: "mailto:someone@example.com")!))
    #expect(of("file://localhost/tmp") == .directory("/tmp"))
    #expect(of("file://MyMac/tmp") == .directory("/tmp"), "the machine's first label")
    #expect(of("file://other/tmp") == nil, "another machine's path")
    for refused in ["javascript:alert(1)", "ftp://example.com", "https://a@b", "file:////h/s/x",
                    "file:///no/such/path", "/etc/passwd", "not a url", ""] {
        #expect(of(refused) == nil, "\(refused)")
    }
}

@Test func aLocalDirectoryOpensAndAFileIsOnlyRevealed() throws {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("scull links \(UUID())")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let script = dir.appendingPathComponent("run me.command")
    try Data("#!/bin/sh\n".utf8).write(to: script)
    #expect(LinkTarget.of(dir.absoluteString) == .directory(dir.path))
    #expect(LinkTarget.of(script.absoluteString) == .file(script.path))
}

/// A view over a terminal with no child showing `text`, in a window of its
/// own, with opened links caught in `opened`.
@MainActor
private final class LinkScreen {
    let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 100))
    let window: NSWindow
    var opened: [LinkTarget] = []

    init(_ text: String) throws {
        let session = try TerminalSession(cols: 12, rows: 2)
        _ = session.feed(Array(text.utf8))
        session.update()
        view.session = session
        view.setFrameSize(NSSize(width: 12.5 * view.cellWidth, height: 2.5 * view.cellHeight))
        window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = view
        view.launch = { [unowned self] in opened.append($0) }
    }

    func event(_ type: NSEvent.EventType, col: Int, _ flags: NSEvent.ModifierFlags = []) -> NSEvent {
        let point = view.convert(NSPoint(x: (Double(col) + 0.5) * view.cellWidth, y: 0.5 * view.cellHeight), to: nil)
        return NSEvent.mouseEvent(with: type, location: point, modifierFlags: flags, timestamp: 0,
                                  windowNumber: window.windowNumber, context: nil, eventNumber: 0,
                                  clickCount: 1, pressure: 1)!
    }

    func click(col: Int, _ flags: NSEvent.ModifierFlags = []) {
        view.mouseDown(with: event(.leftMouseDown, col: col, flags))
        view.mouseUp(with: event(.leftMouseUp, col: col, flags))
    }

    func linked(col: Int) -> Bool { view.session!.cell(row: 0, col: col)!.flags & UInt8(TT_CELL_LINK) != 0 }
}

private func link(_ uri: String, _ text: String) -> String { "\u{1b}]8;;\(uri)\u{1b}\\\(text)\u{1b}]8;;\u{1b}\\" }

@MainActor
@Test func cmdUnderlinesTheLinkUnderThePointer() throws {
    let screen = try LinkScreen("a" + link("https://example.com", "bc") + " d")
    screen.view.mouseMoved(with: screen.event(.mouseMoved, col: 1))
    #expect(!screen.linked(col: 1), "not without Cmd")
    screen.view.mouseMoved(with: screen.event(.mouseMoved, col: 2, .command))
    #expect(screen.linked(col: 1) && screen.linked(col: 2) && !screen.linked(col: 0) && !screen.linked(col: 4))
    screen.view.mouseMoved(with: screen.event(.mouseMoved, col: 4, .command))
    #expect(!screen.linked(col: 1), "off the link")
    screen.view.mouseMoved(with: screen.event(.mouseMoved, col: 1, .command))
    screen.view.hoverLink(at: nil, flags: [])
    #expect(!screen.linked(col: 1), "Cmd let go")
}

@MainActor
@Test func cmdClickOpensAllowedLinksAndOnlyThose() throws {
    let screen = try LinkScreen(link("https://example.com/x", "ab") + link("javascript:alert(1)", "cd") + "ef")
    screen.click(col: 1)
    #expect(screen.opened.isEmpty, "a plain click selects")
    screen.click(col: 1, .command)
    #expect(screen.opened == [.web(URL(string: "https://example.com/x")!)])
    screen.click(col: 2, .command)
    #expect(screen.opened.count == 1, "a refused scheme opens nothing")
    #expect(screen.view.session?.hasSelection == false, "and the click is not a selection")
    screen.click(col: 5, .command)
    #expect(screen.opened.count == 1, "no link there")
}
