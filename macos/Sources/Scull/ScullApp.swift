// One window running the user's shell. The app ends with its window, as
// the window ends with the shell.

import AppKit
import ScullKit
import SwiftUI

@main
struct ScullApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate

    var body: some Scene {
        Window("Scull", id: "main") {
            TerminalSurface()
                .frame(minWidth: 200, minHeight: 100)
        }
        .defaultSize(width: 720, height: 460)

        Settings { SettingsView() }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
