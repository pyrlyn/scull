// The settings window: each control writes its key into the config file
// through the core, which validates it and applies it everywhere at once.
// Editing the file by hand moves the controls the same way.

import AppKit
import ScullKit
import SwiftUI

struct SettingsView: View {
    private let config = ScullConfig.shared
    private let families = FontSet.monospacedFamilies()
    private let schemes = ["scull-dark", "scull-light", "solarized-dark"]

    var body: some View {
        let settings = config.settings
        Form {
            Picker("Font", selection: binding(\.fontFamily, "font.family")) {
                Text("System monospace").tag("")
                if !settings.fontFamily.isEmpty, !families.contains(settings.fontFamily) {
                    Text(settings.fontFamily).tag(settings.fontFamily)
                }
                ForEach(families, id: \.self) { Text($0).tag($0) }
            }
            Stepper(value: binding(\.fontSize, "font.size"), in: 4...200, step: 0.5) {
                Text("Size \(settings.fontSize, format: .number.precision(.fractionLength(0...1))) pt")
            }
            Picker("Colour scheme", selection: binding(\.scheme, "colors.scheme")) {
                ForEach(schemes, id: \.self) { Text($0) }
            }
            TextField("Scrollback rows", value: binding(\.scrollback, "scrollback"), format: .number.grouping(.never))
            Text("Applies to terminals opened afterwards.").font(.caption).foregroundStyle(.secondary)
            if !settings.error.isEmpty {
                Text(settings.error).foregroundStyle(.red).textSelection(.enabled)
            }
            Button("Show config file") {
                NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: settings.path)])
            }
            .disabled(settings.path.isEmpty)
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .fixedSize(horizontal: false, vertical: true)
    }

    /// A control's binding: reads the core's value, writes by key. A value the
    /// core refuses leaves the setting, and the control snaps back.
    private func binding<V: LosslessStringConvertible & Equatable>(
        _ path: KeyPath<ScullSettings, V>, _ key: String
    ) -> Binding<V> {
        Binding(get: { config.settings[keyPath: path] },
                set: { config.set(key, String($0)) })
    }
}
