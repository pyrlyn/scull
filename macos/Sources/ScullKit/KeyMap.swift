// Mac key codes and modifier flags to the core's key vocabulary. The core
// names keys by position (the US-layout character of a main-block key, a
// TT_KEY_* code otherwise), so the mapping works from the hardware key
// code, never from the layout's text, which arrives separately.
// The key code table follows Carbon's kVK_* constants (Events.h).

import AppKit
import CScull

public enum KeyMap {
    /// The core's key for a Mac virtual key code, or nil for a key it has
    /// no name for.
    public static func key(forKeyCode code: UInt16) -> UInt32? {
        if let special = special[code] { return special }
        return mainBlock[code].map { $0.unicodeScalars.first.map(\.value) ?? 0 }
    }

    /// TT_MOD_* bits for AppKit's modifier flags. Option is Alt and Command
    /// is Super, as the protocol's macOS notes say.
    public static func mods(_ flags: NSEvent.ModifierFlags) -> UInt8 {
        var mods: UInt8 = 0
        let pairs: [(NSEvent.ModifierFlags, Int32)] = [
            (.shift, TT_MOD_SHIFT), (.option, TT_MOD_ALT), (.control, TT_MOD_CTRL),
            (.command, TT_MOD_SUPER), (.capsLock, TT_MOD_CAPS_LOCK),
        ]
        for (flag, bit) in pairs where flags.contains(flag) {
            mods |= UInt8(bit)
        }
        return mods
    }

    // The main block in kVK_ANSI_* order; gaps are keys handled elsewhere
    // or absent from the US layout.
    private static let mainBlock: [UInt16: Character] = {
        var table: [UInt16: Character] = [:]
        let spans: [(UInt16, String)] = [
            (0x00, "asdfhgzxcv"),
            (0x0B, "bqweryt123465=97-80]ou[ip"),
            (0x25, "lj'k;\\,/nm."),
            (0x31, " `"),
        ]
        for (start, chars) in spans {
            for (offset, char) in chars.enumerated() {
                table[start + UInt16(offset)] = char
            }
        }
        return table
    }()

    private static let special: [UInt16: UInt32] = {
        var table: [UInt16: UInt32] = [:]
        // Escape to End, in the order TT_KEY_ESCAPE's codes follow.
        let editing: [UInt16] = [
            0x35, 0x24, 0x30, 0x33, 0x72, 0x75, 0x7B, 0x7C, 0x7E, 0x7D, 0x74, 0x79, 0x73, 0x77,
        ]
        let functions: [UInt16] = [
            0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D,
            0x67, 0x6F, 0x69, 0x6B, 0x71, 0x6A, 0x40, 0x4F, 0x50, 0x5A,
        ]
        // Keypad 0-9, Decimal, Divide, Multiply, Subtract, Add, Enter, Equal.
        let keypad: [UInt16] = [
            0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C,
            0x41, 0x4B, 0x43, 0x4E, 0x45, 0x4C, 0x51,
        ]
        for (base, codes) in [(TT_KEY_ESCAPE, editing), (TT_KEY_F1, functions), (TT_KEY_KP_0, keypad)] {
            for (offset, code) in codes.enumerated() {
                table[code] = UInt32(base) + UInt32(offset)
            }
        }
        return table
    }()
}
