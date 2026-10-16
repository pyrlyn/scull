// The SwiftUI face of PaneHostView. Each window or tab gets its own host,
// and so its own pane tree and terminals.

import SwiftUI

public struct PaneSurface: NSViewRepresentable {
    public init() {}

    public func makeNSView(context: Context) -> PaneHostView {
        PaneHostView(frame: .zero)
    }

    public func updateNSView(_ view: PaneHostView, context: Context) {}
}
