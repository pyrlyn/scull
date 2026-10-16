// The user's configuration as the core reads it: one tt_config, polled on
// the main actor like the frame. The core watches the file; its wakeup only
// posts to the main queue, where the settings are read and observers told.

import CScull
import Foundation
import Observation

/// What a key binding does; the numbers are the core's TT_ACTION_* values.
public enum ScullAction: UInt8, Sendable {
    case paste = 1, fontLarger, fontSmaller, fontReset
    case scrollPageUp, scrollPageDown, scrollToTop, scrollToBottom
}

public struct ScullKeybind: Equatable, Sendable {
    public var key: UInt32
    public var mods: UInt8
    public var action: ScullAction
}

public struct ScullSettings: Equatable, Sendable {
    /// Empty for the system's monospace font.
    public var fontFamily = ""
    public var fontSize = 13.0
    public var scrollback: UInt32 = 10_000
    public var scheme = "scull-dark"
    public var foreground: UInt32 = 0xE5E5E5
    public var background: UInt32 = 0x141414
    public var cursor: UInt32 = 0xE5E5E5
    public var ansi: [UInt32] = []
    public var keybinds: [ScullKeybind] = []
    /// Why the file was refused, with its name; empty when it was fine.
    public var error = ""
    public var path = ""
}

@MainActor
@Observable
public final class ScullConfig {
    /// Posted on the main actor when the settings changed.
    public static let didChange = Notification.Name("ScullConfigDidChange")

    /// The app's configuration. `-ScullConfig <path>` picks another file in
    /// debug builds, so a scripted run does not touch the user's.
    public static let shared: ScullConfig = {
        #if DEBUG
        return ScullConfig(path: UserDefaults.standard.string(forKey: "ScullConfig") ?? "")
        #else
        return ScullConfig()
        #endif
    }()

    public private(set) var settings = ScullSettings()
    @ObservationIgnored private var config: OpaquePointer?
    @ObservationIgnored private var waker: Unmanaged<Waker>?

    /// Opens the file at `path`, or the default place when empty. Without a
    /// usable place the built-in defaults stay and nothing is watched.
    public init(path: String = "") {
        let waker = Unmanaged.passRetained(Waker { [weak self] in self?.refresh() })
        var options = tt_config_options()
        options.struct_size = UInt32(MemoryLayout<tt_config_options>.size)
        options.abi_version = UInt32(TT_ABI_VERSION)
        options.wakeup = { userdata in
            guard let userdata else { return }
            Unmanaged<Waker>.fromOpaque(userdata).takeUnretainedValue().fire()
        }
        options.userdata = waker.toOpaque()
        var out: OpaquePointer?
        let status = withStrings([path]) { strs in
            options.path = strs[0]
            return tt_config_new(&options, &out)
        }
        guard status == TT_OK, let out else {
            waker.release()
            return
        }
        (config, self.waker) = (out, waker)
        refresh()
    }

    isolated deinit {
        // Joins the watcher, so no wakeup can follow the release.
        tt_config_free(config)
        waker?.release()
    }

    /// Reads the core's current settings; observers hear only of a change.
    private func refresh() {
        guard let config else { return }
        var view = tt_config_view()
        view.struct_size = UInt32(MemoryLayout<tt_config_view>.size)
        guard tt_config_poll(config, &view) == TT_OK, view.updated != 0 else { return }
        settings = Self.settings(from: view)
        NotificationCenter.default.post(name: Self.didChange, object: self)
    }

    /// Changes one setting in the file (`font.size`, `colors.scheme`, ...);
    /// an empty value removes it. The core re-reads and wakes us, so the
    /// new value is in `settings` on return. False when the core refused it.
    @discardableResult
    public func set(_ key: String, _ value: String) -> Bool {
        guard let config else { return false }
        let status = withStrings([key, value]) { tt_config_set(config, $0[0], $0[1]) }
        refresh()
        return status == TT_OK
    }

    private static func settings(from view: tt_config_view) -> ScullSettings {
        let binds = (0..<view.keybinds_len).compactMap { i -> ScullKeybind? in
            let bind = view.keybinds[i]
            return ScullAction(rawValue: bind.action).map { ScullKeybind(key: bind.key, mods: bind.mods, action: $0) }
        }
        return ScullSettings(
            fontFamily: string(view.font_family), fontSize: Double(view.font_size),
            scrollback: view.scrollback, scheme: string(view.scheme), foreground: view.foreground,
            background: view.background, cursor: view.cursor,
            ansi: withUnsafeBytes(of: view.ansi) { Array($0.bindMemory(to: UInt32.self)) },
            keybinds: binds, error: string(view.error), path: string(view.path))
    }

    private static func string(_ s: tt_str) -> String {
        guard let ptr = s.ptr else { return "" }
        return String(decoding: UnsafeBufferPointer(start: ptr, count: s.len), as: UTF8.self)
    }
}
