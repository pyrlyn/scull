// Prompt themes print Powerline separators and Nerd Font icons from the
// Private Use Area. No system font has them, so the app ships the Nerd
// Fonts symbols cut and draws with it what the face itself lacks. A
// cascade list would be simpler, but CTLine ignores it on the system
// monospaced font, which is the default face.

import AppKit
import CoreText

struct SymbolFont {
    static let file = "SymbolsNerdFontMono-Regular"
    static let name = "SymbolsNFM"

    // Registered once per process; the app bundle carries the file in
    // Contents/Resources. A missing or broken file only loses the icons.
    private static let registered: Bool = {
        guard let url = Bundle.main.url(forResource: file, withExtension: "ttf") else { return false }
        return register(url)
    }()

    static func register(_ url: URL) -> Bool {
        var error: Unmanaged<CFError>?
        if CTFontManagerRegisterFontsForURL(url as CFURL, .process, &error) { return true }
        // Registered already by an earlier call: the font is there to use.
        let code = error.map { CFErrorGetCode($0.takeRetainedValue()) }
        return code == CTFontManagerError.alreadyRegistered.rawValue
    }

    let font: CTFont?

    init(size: CGFloat) {
        _ = Self.registered
        let font = CTFontCreateWithName(Self.name as CFString, size, nil)
        // A name CoreText cannot find resolves to some other font, which
        // must not take over from the system fallback.
        self.font = CTFontCopyPostScriptName(font) as String == Self.name ? font : nil
    }

    // `text` with `attributes`, its font switched to the symbols font on
    // every character `face` has no glyph for and the symbols font has.
    func string(_ text: String, face: NSFont,
                attributes: [NSAttributedString.Key: Any]) -> NSAttributedString {
        let string = NSMutableAttributedString(string: text, attributes: attributes)
        guard let font else { return string }
        var offset = 0
        for scalar in text.unicodeScalars {
            let units = Array(String(scalar).utf16)
            // Every monospaced face covers ASCII, so most text skips the lookups.
            if !scalar.isASCII, !Self.has(face, units), Self.has(font, units) {
                string.addAttribute(NSAttributedString.Key(kCTFontAttributeName as String), value: font,
                                    range: NSRange(location: offset, length: units.count))
            }
            offset += units.count
        }
        return string
    }

    private static func has(_ font: CTFont, _ units: [UniChar]) -> Bool {
        var glyphs = [CGGlyph](repeating: 0, count: units.count)
        return CTFontGetGlyphsForCharacters(font, units, &glyphs, units.count)
    }
}
