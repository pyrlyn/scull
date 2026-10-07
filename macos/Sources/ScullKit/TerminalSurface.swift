// The SwiftUI face of TerminalView. The view owns the terminal, so
// SwiftUI only creates it; there is no state to push back into it.

import SwiftUI

public struct TerminalSurface: NSViewRepresentable {
    public init() {}

    public func makeNSView(context: Context) -> TerminalView {
        TerminalView(frame: .zero)
    }

    public func updateNSView(_ view: TerminalView, context: Context) {}
}
