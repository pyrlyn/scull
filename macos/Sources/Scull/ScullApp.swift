// Windows of split panes, each running the user's shell. Tabs are the
// system's window tabs; the app ends with its last window, as a window
// ends with its last shell.

import AppKit
import ScullKit
import SwiftUI

@main
struct ScullApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        WindowGroup("Scull", id: "terminal") {
            PaneSurface()
                .frame(minWidth: 200, minHeight: 100)
        }
        .defaultSize(width: 720, height: 460)
        .commands { PaneCommands() }
    }
}

/// Sends the action up the responder chain from the key window's focused
/// terminal; a window with no pane host takes `fallback` instead.
@MainActor
private func send(_ action: Selector, fallback: () -> Void = {}) {
    if !NSApp.sendAction(action, to: nil, from: nil) { fallback() }
}

struct PaneCommands: Commands {
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(after: .newItem) {
            Button("New Tab") {
                PaneHostView.tabAnchor = NSApp.keyWindow
                openWindow(id: "terminal")
            }
            .keyboardShortcut("t")
        }
        // The system's Close is replaced: the shortcut closes a pane first.
        CommandGroup(replacing: .saveItem) {
            Button("Close Pane") {
                send(#selector(PaneHostView.closePane)) { NSApp.keyWindow?.performClose(nil) }
            }
            .keyboardShortcut("w")
            Button("Close Window") { NSApp.keyWindow?.performClose(nil) }
                .keyboardShortcut("w", modifiers: [.command, .shift])
        }
        CommandMenu("Pane") {
            Button("Split Right") { send(#selector(PaneHostView.splitRight)) }
                .keyboardShortcut("d")
            Button("Split Down") { send(#selector(PaneHostView.splitDown)) }
                .keyboardShortcut("d", modifiers: [.command, .shift])
            Divider()
            Button("Next Pane") { send(#selector(PaneHostView.focusNextPane)) }
                .keyboardShortcut("]")
            Button("Previous Pane") { send(#selector(PaneHostView.focusPreviousPane)) }
                .keyboardShortcut("[")
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
