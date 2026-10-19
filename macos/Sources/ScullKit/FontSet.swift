// The four faces of the configured font and the cell they imply. A family
// that is missing or not monospaced falls back to the system's monospaced
// font: a proportional face would break the grid.

import AppKit

public struct FontSet {
    public let regular: NSFont
    public let bold: NSFont
    public let italic: NSFont
    public let boldItalic: NSFont
    public let cellWidth: CGFloat
    public let cellHeight: CGFloat

    public init(family: String, size: CGFloat) {
        let regular = Self.face(family: family, size: size)
        let manager = NSFontManager.shared
        // Families without a bold or italic cut keep the regular face.
        func styled(_ font: NSFont, _ trait: NSFontTraitMask) -> NSFont {
            let converted = manager.convert(font, toHaveTrait: trait)
            return converted.isFixedPitch ? converted : font
        }
        let bold = styled(regular, .boldFontMask)
        self.regular = regular
        self.bold = bold
        italic = styled(regular, .italicFontMask)
        boldItalic = styled(bold, .italicFontMask)
        // Advances, not rounded cells, so CoreText's glyphs land on the grid.
        cellWidth = ("M" as NSString).size(withAttributes: [.font: regular]).width
        cellHeight = ceil(regular.ascender - regular.descender + regular.leading)
    }

    private static func face(family: String, size: CGFloat) -> NSFont {
        if !family.isEmpty {
            let descriptor = NSFontDescriptor(fontAttributes: [.family: family])
            if let font = NSFont(descriptor: descriptor, size: size), font.isFixedPitch,
               font.familyName?.caseInsensitiveCompare(family) == .orderedSame {
                return font
            }
        }
        return NSFont.monospacedSystemFont(ofSize: size, weight: .regular)
    }

    /// The installed families that are monospaced, for a picker.
    public static func monospacedFamilies() -> [String] {
        let manager = NSFontManager.shared
        return manager.availableFontFamilies.filter { family in
            guard let member = manager.availableMembers(ofFontFamily: family)?.first,
                  let name = member.first as? String else { return false }
            return NSFont(name: name, size: 12)?.isFixedPitch ?? false
        }
    }
}
