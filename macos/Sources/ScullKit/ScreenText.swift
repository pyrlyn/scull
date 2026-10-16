// The screen's text as accessibility clients index it: one string, a line
// per row, offsets in UTF-16. The geometry is the frame's cells; this only
// maps between offsets, lines and columns.

import Foundation

struct ScreenText {
    let string: String
    /// The UTF-16 offset each line starts at; never empty.
    private let starts: [Int]

    init(_ string: String) {
        self.string = string
        var starts = [0]
        var offset = 0
        for unit in string.utf16 {
            offset += 1
            if unit == 0x0A { starts.append(offset) }
        }
        self.starts = starts
    }

    var length: Int { (string as NSString).length }

    var lineCount: Int { starts.count }

    /// The line holding `index`; past the end it is the last line.
    func line(for index: Int) -> Int {
        let after = starts.partitionPoint { $0 > max(0, index) }
        return after - 1
    }

    /// The line's text with its newline, as NSTextView answers it.
    func range(forLine line: Int) -> NSRange? {
        guard starts.indices.contains(line) else { return nil }
        let end = line + 1 < starts.count ? starts[line + 1] : length
        return NSRange(location: starts[line], length: end - starts[line])
    }

    func substring(_ range: NSRange) -> String? {
        guard range.location != NSNotFound, range.location >= 0, NSMaxRange(range) <= length else { return nil }
        return (string as NSString).substring(with: range)
    }

    /// Where each character of `line` starts, as (UTF-16 offset, column),
    /// then the line's end. `width` answers the row's cell width at a
    /// column, so a wide character takes its two columns.
    func columns(line: Int, width: (Int) -> Int) -> [(index: Int, col: Int)] {
        guard let range = range(forLine: line), let text = substring(range) else { return [] }
        var stops: [(index: Int, col: Int)] = []
        var (index, col) = (range.location, 0)
        for character in text where character != "\n" {
            stops.append((index, col))
            index += character.utf16.count
            col += max(1, width(col))
        }
        stops.append((index, col))
        return stops
    }

    /// The offset of the character at `col` of `line`, or the line's end.
    func index(line: Int, col: Int, width: (Int) -> Int) -> Int {
        columns(line: line, width: width).last { $0.col <= col }?.index ?? length
    }
}

private extension Array {
    /// The first index whose element satisfies `belongs`, which must hold
    /// for a suffix of the array.
    func partitionPoint(_ belongs: (Element) -> Bool) -> Int {
        var (low, high) = (0, count)
        while low < high {
            let mid = (low + high) / 2
            if belongs(self[mid]) { high = mid } else { low = mid + 1 }
        }
        return low
    }
}
