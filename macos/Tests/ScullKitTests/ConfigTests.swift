import AppKit
import CScull
import Foundation
import Testing
@testable import ScullKit

private func scratchConfig() throws -> (path: String, cleanup: () -> Void) {
    let dir = FileManager.default.temporaryDirectory.appendingPathComponent("scull-cfg-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return (dir.appendingPathComponent("config.toml").path, { try? FileManager.default.removeItem(at: dir) })
}

/// Lets the main queue run, where the core's wakeups land, until `done`.
@MainActor
private func wait(_ what: String, _ done: () -> Bool) async throws {
    for _ in 0..<1000 where !done() { try await Task.sleep(for: .milliseconds(20)) }
    #expect(done(), "timed out waiting for \(what)")
}

@MainActor
@Test func configStartsAsTheDefaultsAndSetGoesThroughTheCore() throws {
    let (path, cleanup) = try scratchConfig()
    defer { cleanup() }
    let config = ScullConfig(path: path)
    #expect(config.settings.fontSize == 13 && config.settings.error.isEmpty)
    #expect(config.settings.path == path)
    #expect(config.settings.ansi.count == 16)
    #expect(config.settings.keybinds.contains { $0.action == .paste && $0.key == 0x76 && $0.mods == UInt8(TT_MOD_SUPER) })
    #expect(config.set("font.size", "17"))
    #expect(config.settings.fontSize == 17)
    #expect(config.set("colors.scheme", "scull-light"))
    #expect(config.settings.background == 0xFAFAFA)
    #expect(!config.set("font.size", "9999"))
    #expect(config.settings.fontSize == 17)
    #expect(config.set("font.size", ""))
    #expect(config.settings.fontSize == 13)
}

@MainActor
@Test func everySchemeNamedInTheSettingsViewIsAcceptedByTheCore() throws {
    let (path, cleanup) = try scratchConfig()
    defer { cleanup() }
    let config = ScullConfig(path: path)
    for name in ["scull-dark", "scull-light", "solarized-dark"] {
        #expect(config.set("colors.scheme", name), "\(name)")
    }
}

@MainActor
@Test func editingTheFileChangesTheSettingsWithoutARestart() async throws {
    let (path, cleanup) = try scratchConfig()
    defer { cleanup() }
    let config = ScullConfig(path: path)
    var notified = 0
    let token = NotificationCenter.default.addObserver(forName: ScullConfig.didChange, object: config, queue: .main) { _ in
        notified += 1
    }
    defer { NotificationCenter.default.removeObserver(token) }
    try "[font]\nsize = 21\nfamily = \"Menlo\"\n[colors]\nbackground = \"#102030\"\n".write(toFile: path, atomically: true, encoding: .utf8)
    try await wait("the new font") { config.settings.fontSize == 21 }
    #expect(config.settings.fontFamily == "Menlo")
    #expect(config.settings.background == 0x102030)
    #expect(notified >= 1)
    // A mistake keeps what worked and says what is wrong.
    try "[font]\nsize = 1\n".write(toFile: path, atomically: true, encoding: .utf8)
    try await wait("the error") { !config.settings.error.isEmpty }
    #expect(config.settings.fontSize == 21)
    #expect(config.settings.error.contains("config.toml"))
}

@Test func paletteTakesTheConfiguredColours() {
    var settings = ScullSettings()
    settings.foreground = 0x111111
    settings.background = 0x222222
    settings.cursor = 0x333333
    settings.ansi = (0..<16).map { UInt32($0) * 0x010101 }
    let palette = Palette(settings)
    #expect(palette.foreground == 0x111111 && palette.background == 0x222222 && palette.cursor == 0x333333)
    #expect(palette.rgb(UInt32(TT_COLOR_INDEXED) | 5, fallback: 0) == 0x050505)
    #expect(palette.rgb(UInt32(TT_COLOR_INDEXED) | 16, fallback: 1) == 0x000000, "the cube stays xterm's")
}

@MainActor
@Test func fontSetUsesTheFamilyAndFallsBackToMonospace() {
    let menlo = FontSet(family: "Menlo", size: 15)
    #expect(menlo.regular.familyName == "Menlo" && menlo.regular.pointSize == 15)
    #expect(menlo.bold.fontDescriptor.symbolicTraits.contains(.bold))
    // A proportional or unknown family would break the grid.
    for family in ["Helvetica Neue", "No Such Font"] {
        let set = FontSet(family: family, size: 15)
        #expect(set.regular.isFixedPitch, "\(family)")
    }
    #expect(FontSet(family: "Menlo", size: 30).cellWidth > menlo.cellWidth)
    #expect(FontSet.monospacedFamilies().contains("Menlo"))
}
