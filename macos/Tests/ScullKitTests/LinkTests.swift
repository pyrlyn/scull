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
