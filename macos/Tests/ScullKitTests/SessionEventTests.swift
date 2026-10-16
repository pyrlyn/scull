import AppKit
import CScull
import Testing
@testable import ScullKit

/// A pasteboard of its own, so a test never touches the user's clipboard.
@MainActor
private func privatePasteboard() -> NSPasteboard {
    NSPasteboard(name: NSPasteboard.Name("org.scull.test.\(UUID().uuidString)"))
}

@MainActor
@Test func titlesAndTheDirectoryAreKeptOnTheSession() throws {
    let session = try TerminalSession(cols: 10, rows: 3)
    #expect(session.title == nil && session.workingDirectory == nil)
    _ = session.feed(Array("\u{1b}]2;vim 世\u{7}\u{1b}]1;icon only\u{7}\u{1b}]7;file://host/tmp/a%20b\u{7}".utf8))
    var bells = 0
    #expect(!session.drainEvents { bells += 1 })
    #expect(session.title == "vim 世", "an icon name is not the window title")
    #expect(session.workingDirectory == "/tmp/a b")
    _ = session.feed(Array("\u{1b}]0;both\u{7}\u{1b}]7;https://example.com/x\u{7}\u{7}".utf8))
    #expect(!session.drainEvents { bells += 1 })
    #expect(session.title == "both")
    #expect(session.workingDirectory == "/tmp/a b", "only a file URL is a directory")
    #expect(bells == 1)
}

@MainActor
@Test func clipboardWritesReachThePasteboardAndReadsAreDenied() throws {
    let session = try TerminalSession(cols: 10, rows: 3)
    let pasteboard = privatePasteboard()
    defer { pasteboard.releaseGlobally() }
    session.pasteboard = pasteboard
    // "aGk=" is "hi"; "/w==" is a lone 0xFF, which is not text.
    _ = session.feed(Array("\u{1b}]52;c;aGk=\u{7}\u{1b}]52;c;/w==\u{7}".utf8))
    _ = session.drainEvents {}
    #expect(pasteboard.string(forType: .string) == "hi")
    // Clipboard requests are numbered from 1: the two writes were 1 and 2.
    _ = session.feed(Array("\u{1b}]52;c;?\u{7}".utf8))
    _ = session.drainEvents {}
    #expect(tt_term_clipboard_deny(session.term, 3) == TT_INVALID, "the session denied it")
    _ = session.feed(Array("\u{1b}]52;c;?\u{7}".utf8))
    #expect(tt_term_clipboard_deny(session.term, 4) == TT_OK, "an unpolled read is still open")
    #expect(pasteboard.string(forType: .string) == "hi", "a read leaves the pasteboard alone")
}
