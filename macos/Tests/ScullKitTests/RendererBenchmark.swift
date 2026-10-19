// Renderer throughput on the T3 input: the same 1000000 bytes as
// `scull-bench`, fed through the real core with the frame updated and
// drawn offscreen. Runs only when `SCULL_RENDER_BENCH` names the file for
// the results (`just macos-bench`), since a debug build's numbers mean
// nothing.

import AppKit
import Foundation
import Testing
@testable import ScullKit

/// `scull_bench::fixed_input`: A to Z, a line feed closing every 80 bytes.
private func fixedInput() -> [UInt8] {
    var out: [UInt8] = []
    out.reserveCapacity(1_000_000)
    var letter = 0
    while out.count < 1_000_000 {
        if (out.count + 1) % 80 == 0 {
            out.append(UInt8(ascii: "\n"))
        } else {
            out.append(UInt8(ascii: "A") + UInt8(letter % 26))
            letter += 1
        }
    }
    return out
}

private func seconds(_ d: Duration) -> Double {
    Double(d.components.seconds) + Double(d.components.attoseconds) / 1e18
}

@MainActor
@Test(.enabled(if: ProcessInfo.processInfo.environment["SCULL_RENDER_BENCH"] != nil))
func rendererThroughput() throws {
    let font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
    let cell = (w: ("M" as NSString).size(withAttributes: [.font: font]).width,
                h: ceil(font.ascender - font.descender + font.leading))
    let size = CGSize(width: cell.w * 80, height: cell.h * 24)
    let renderer = try #require(MetalRenderer(font: font, cellWidth: cell.w, cellHeight: cell.h, palette: Palette()))
    let input = fixedInput()
    let mib = Double(input.count) / 1_048_576
    let clock = ContinuousClock()
    var lines = ["| case | chunk | frames | seconds | MiB/s | ms per frame |", "| --- | --- | --- | --- | --- | --- |"]

    for chunk in [4096, 65536] {
        for draws in [false, true] {
            let session = try TerminalSession(cols: 80, rows: 24)
            // Warm the atlas and the shaper so the run times steady state.
            _ = session.update()
            _ = renderer.renderOffscreen(session, size: size, scale: 2, focused: true)
            var frames = 0
            let elapsed = clock.measure {
                for start in stride(from: 0, to: input.count, by: chunk) {
                    _ = session.feed(Array(input[start..<min(start + chunk, input.count)]))
                    guard session.update(), draws else { continue }
                    _ = renderer.renderOffscreen(session, size: size, scale: 2, focused: true)
                    frames += 1
                }
            }
            let s = seconds(elapsed)
            lines.append(String(format: "| %@ | %d | %d | %.6f | %.2f | %@ |", draws ? "feed, update, render" : "feed, update",
                                chunk, frames, s, mib / s, frames > 0 ? String(format: "%.3f", s * 1000 / Double(frames)) : "-"))
        }
    }

    // One frame's cost when one row changes (an echo) and when every row
    // does (a full screen of new text).
    let session = try TerminalSession(cols: 80, rows: 24)
    _ = session.feed(Array(input[0..<(80 * 24)]))
    _ = session.update()
    _ = renderer.renderOffscreen(session, size: size, scale: 2, focused: true)
    for (name, bytes) in [("one row", Array("x".utf8)), ("all rows", Array(input[0..<(80 * 24)]))] {
        let count = 200
        let elapsed = clock.measure {
            for _ in 0..<count {
                _ = session.feed(bytes)
                _ = session.update()
                _ = renderer.renderOffscreen(session, size: size, scale: 2, focused: true)
            }
        }
        lines.append(String(format: "| %@ changed | - | %d | %.6f | - | %.3f |", name, count, seconds(elapsed),
                            seconds(elapsed) * 1000 / Double(count)))
    }
    let path = try #require(ProcessInfo.processInfo.environment["SCULL_RENDER_BENCH"])
    try (lines.joined(separator: "\n") + "\n").write(toFile: path, atomically: true, encoding: .utf8)
}
