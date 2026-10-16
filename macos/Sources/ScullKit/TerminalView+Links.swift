// OSC 8 links: Cmd+click opens one, and while Cmd is held the link under
// the pointer is underlined. The core finds and marks the link; its URI is
// whatever the program wrote, so the core's tt_link_target decides what
// may open, and NSWorkspace opens it, never a shell.

import AppKit
import CScull

/// What a click on a link may do, as `tt_link_target` sorted it.
enum LinkTarget: Equatable {
    case web(URL)
    case mail(URL)
    /// A directory on this Mac: it opens in Finder.
    case directory(String)
    /// Any other file on this Mac: Finder only selects it, so a click never
    /// runs something the program wrote.
    case file(String)

    /// The target of `uri`; nil for a link that may not open. `machine` is
    /// the name a file URL may give as its host.
    static func of(_ uri: String, machine: String = localName) -> LinkTarget? {
        var uri = uri, machine = machine
        var kind: UInt32 = 0
        var buf = [UInt8](repeating: 0, count: 256)
        var len = 0
        let sort = {
            uri.withUTF8 { u in
                machine.withUTF8 { m in
                    buf.withUnsafeMutableBufferPointer {
                        tt_link_target(tt_str(ptr: u.baseAddress, len: u.count), tt_str(ptr: m.baseAddress, len: m.count),
                                       &kind, $0.baseAddress, $0.count, &len)
                    }
                }
            }
        }
        var status = sort()
        if status == TT_FULL {
            buf = [UInt8](repeating: 0, count: len)
            status = sort()
        }
        guard status == TT_OK else { return nil }
        let target = String(decoding: buf.prefix(len), as: UTF8.self)
        switch Int32(kind) {
        case TT_LINK_WEB: return URL(string: target).map(LinkTarget.web)
        case TT_LINK_MAIL: return URL(string: target).map(LinkTarget.mail)
        case TT_LINK_DIRECTORY: return .directory(target)
        case TT_LINK_FILE: return .file(target)
        default: return nil
        }
    }

    /// Hands the target to the workspace: a page or address opens, a
    /// directory opens in Finder, a file is selected there.
    @MainActor func launch() {
        switch self {
        case .web(let url), .mail(let url): NSWorkspace.shared.open(url)
        // Rooted at the path rather than opened: an .app bundle is a
        // directory too, and opening it would run it.
        case .directory(let path): NSWorkspace.shared.selectFile(nil, inFileViewerRootedAtPath: path)
        case .file(let path): NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
        }
    }

    /// This Mac's name as shells write it into file URLs; the core also
    /// takes its first label.
    static let localName: String = {
        var buf = [CChar](repeating: 0, count: Int(MAXHOSTNAMELEN) + 1)
        // gethostname, not ProcessInfo.hostName, which may wait on DNS.
        guard gethostname(&buf, buf.count - 1) == 0 else { return "" }
        return String(decoding: buf.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }()
}

extension TerminalView {
    /// Underlines the link at `point` while `flags` hold Cmd, else none.
    func hoverLink(at point: CGPoint?, flags: NSEvent.ModifierFlags) {
        guard let session else { return }
        var id: UInt32?
        if flags.contains(.command), let point, bounds.contains(point) {
            let cell = cell(at: point)
            id = session.linkAt(row: cell.row, col: cell.col)
        }
        guard id != hoveredLink else { return }
        hoveredLink = id
        session.hoverLink(id)
        if session.update() { needsDisplay = true }
    }

    /// Opens the link under a Cmd+click; true when the click was on a link.
    func openLink(_ event: NSEvent) -> Bool {
        guard event.modifierFlags.contains(.command), let session else { return false }
        let cell = cell(at: convert(event.locationInWindow, from: nil))
        guard let id = session.linkAt(row: cell.row, col: cell.col) else { return false }
        // A link that may not open still takes the click: Cmd+click on a
        // link is never meant for the program.
        if let uri = session.linkURI(id), let target = LinkTarget.of(uri) {
            launch(target)
        }
        return true
    }

    public override func flagsChanged(with event: NSEvent) {
        let point = window.map { convert($0.mouseLocationOutsideOfEventStream, from: nil) }
        hoverLink(at: point, flags: event.modifierFlags)
        super.flagsChanged(with: event)
    }
}
