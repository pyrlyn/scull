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

/// A run of selected cells on one screen line; `endCol` is the column after the last one.
struct SelectedSpan: Equatable {
    var line: Int
    var col: Int
    var endCol: Int
}

/// A cell range on the screen, ends inclusive, as the core's selection takes it.
struct ScreenSelection: Equatable {
    var start: (row: Int, col: Int)
    var end: (row: Int, col: Int)

    static func == (a: Self, b: Self) -> Bool { a.start == b.start && a.end == b.end }
}

extension ScreenText {
    /// The value's ranges the selected `spans` cover, one per span. A span begins at the character
    /// its first column belongs to and ends after the one holding its last, so a wide character is
    /// never cut; cells past the trimmed text add nothing, and a span with no text is dropped.
    /// `width(line)(col)` answers a row's cell width at a column.
    func ranges(of spans: [SelectedSpan], width: (Int) -> (Int) -> Int) -> [NSRange] {
        spans.compactMap { span in
            let stops = columns(line: span.line, width: width(span.line))
            guard let start = stops.last(where: { $0.col <= span.col }) else { return nil }
            let end = stops.first { $0.col >= span.endCol } ?? stops[stops.count - 1]
            return end.index > start.index ? NSRange(location: start.index, length: end.index - start.index) : nil
        }
    }

    /// The cells a client's `range` selects: from the first character in it to the last, newlines
    /// skipped. Nil when it holds no character of the text.
    func selection(for range: NSRange, width: (Int) -> (Int) -> Int) -> ScreenSelection? {
        guard range.location != NSNotFound, range.location >= 0, range.location < length else { return nil }
        let text = string as NSString
        let end = min(NSMaxRange(range), length)
        let isText = { (index: Int) in text.character(at: index) != 0x0A }
        guard let first = (range.location..<end).first(where: isText),
              let last = (range.location..<end).last(where: isText),
              let head = cell(at: first, width: width), let tail = cell(at: last, width: width)
        else { return nil }
        return ScreenSelection(start: (head.row, head.col), end: (tail.row, tail.col + tail.cols - 1))
    }

    /// The cell the character holding `index` starts in, with the columns it covers; nil on a line's end.
    private func cell(at index: Int, width: (Int) -> (Int) -> Int) -> (row: Int, col: Int, cols: Int)? {
        let row = line(for: index)
        let stops = columns(line: row, width: width(row))
        guard let k = stops.lastIndex(where: { $0.index <= index }), k + 1 < stops.count else { return nil }
        return (row, stops[k].col, stops[k + 1].col - stops[k].col)
    }
}
