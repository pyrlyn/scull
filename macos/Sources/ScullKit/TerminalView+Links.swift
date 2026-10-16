// OSC 8 links: Cmd+click opens one, and while Cmd is held the link under
// the pointer is underlined. The core finds and marks the link; its URI is
// whatever the program wrote, so LinkPolicy decides what may open, and
// NSWorkspace opens it, never a shell.

import AppKit
import CScull

enum LinkPolicy {
    /// The URL a click on `uri` may open: http and https with a host,
    /// mailto, and a file URL naming this Mac (no host, `localhost` or one
    /// of `localNames`, lowercased). Nil for everything else.
    static func openable(_ uri: String, localNames: Set<String>) -> URL? {
        guard let url = URL(string: uri), let scheme = url.scheme?.lowercased() else { return nil }
        switch scheme {
        case "http", "https":
            return url.host?.isEmpty == false ? url : nil
        case "mailto":
            return url.absoluteString.count > "mailto:".count ? url : nil
        case "file":
            let host = url.host?.lowercased() ?? ""
            guard host.isEmpty || host == "localhost" || localNames.contains(host), !url.path.isEmpty else { return nil }
            // Rebuilt without the host, so nothing reads it as a share.
            return URL(fileURLWithPath: url.path)
        default:
            return nil
        }
    }

    /// This Mac's names as shells write them into file URLs: the full host
    /// name and its first label.
    static let localNames: Set<String> = {
        var buf = [CChar](repeating: 0, count: Int(MAXHOSTNAMELEN) + 1)
        // gethostname, not ProcessInfo.hostName, which may wait on DNS.
        guard gethostname(&buf, buf.count - 1) == 0 else { return [] }
        let name = String(decoding: buf.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self).lowercased()
        return name.isEmpty ? [] : [name, String(name.prefix { $0 != "." })]
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
        if let uri = session.linkURI(id), let url = LinkPolicy.openable(uri, localNames: LinkPolicy.localNames) {
            openURL(url)
        }
        return true
    }

    public override func flagsChanged(with event: NSEvent) {
        let point = window.map { convert($0.mouseLocationOutsideOfEventStream, from: nil) }
        hoverLink(at: point, flags: event.modifierFlags)
        super.flagsChanged(with: event)
    }
}
