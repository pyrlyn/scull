import AppKit
import CScull
import Testing
@testable import ScullKit

private let prompt = "\u{1b}]133;A\u{07}"

@MainActor
private func shell() throws -> (TerminalView, TerminalSession) {
    let view = TerminalView(frame: NSRect(x: 0, y: 0, width: 400, height: 100))
    // A terminal with no child; the prompts come from `feed` only.
    let session = try TerminalSession(cols: 10, rows: 2)
    let marked = ["ant", "bee", "cat", "dog"].map { prompt + $0 }.joined(separator: "\r\n")
    #expect(session.feed(Array(marked.utf8)) == TT_OK)
    session.update()
    view.session = session
    view.setFrameSize(NSSize(width: 10.5 * view.cellWidth, height: 2.5 * view.cellHeight))
    let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView = view
    return (view, session)
}

@MainActor
private func top(_ session: TerminalSession) -> String {
    session.update()
    return session.runs(row: 0).map(\.text).joined()
}

private func command(_ code: UInt16) -> NSEvent {
    NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [.command], timestamp: 0, windowNumber: 0,
                     context: nil, characters: "", charactersIgnoringModifiers: "", isARepeat: false, keyCode: code)!
}

@MainActor
@Test func theViewJumpsBetweenPromptsAndStopsAtTheEnds() throws {
    let (view, session) = try shell()
    #expect(top(session) == "cat")
    view.scrollToPrompt(forward: false)
    #expect(top(session) == "bee")
    view.scrollToPrompt(forward: false)
    #expect(top(session) == "ant")
    view.scrollToPrompt(forward: false)
    #expect(top(session) == "ant", "nothing above the first prompt")
    view.scrollToPrompt(forward: true)
    view.scrollToPrompt(forward: true)
    #expect(top(session) == "cat", "the live screen")
    #expect(session.scrollToPrompt(forward: true) == TT_EMPTY)
}

@MainActor
@Test func commandUpAndDownAreTheDefaultBindings() throws {
    let (view, session) = try shell()
    // The view reads the shared config, whose defaults hold unless the user rebound these keys.
    #expect(view.performKeyEquivalent(with: command(0x7E)))
    #expect(top(session) == "bee")
    #expect(view.performKeyEquivalent(with: command(0x7D)))
    #expect(top(session) == "cat")
}

@MainActor
@Test func theDefaultKeybindsHoldTheJumps() throws {
    let config = ScullConfig(path: FileManager.default.temporaryDirectory
        .appendingPathComponent("scull-prompt-\(UUID().uuidString).toml").path)
    let super_ = UInt8(TT_MOD_SUPER)
    let binds = config.settings.keybinds
    #expect(binds.contains { $0.action == .previousPrompt && $0.mods == super_ })
    #expect(binds.contains { $0.action == .nextPrompt && $0.mods == super_ })
}
