// A debug probe for input latency: it types a key into the core, waits
// for the frame that the echo changes, and takes the times that frame was
// rendered and reached the screen. The start mark is the core key event,
// so AppKit's own event delivery is not in it.

#if DEBUG
import Foundation
import QuartzCore

@MainActor
final class LatencyProbe {
    private let path: String
    private let samples: Int
    private let type: () -> Void
    private var started: CFTimeInterval?
    private var rendered: [Double] = []
    private var presented: [Double] = []
    private var marks = 0

    /// `type` sends one key whose echo changes the frame; the results go to
    /// `path` as text.
    init(path: String, samples: Int = 100, type: @escaping () -> Void) {
        (self.path, self.samples, self.type) = (path, samples, type)
    }

    func next() {
        guard rendered.count < samples else { return finish() }
        started = CACurrentMediaTime()
        type()
    }

    /// The view got a changed frame; the renderer reports on the next one
    /// it draws.
    func frameUpdated(_ renderer: MetalRenderer) {
        guard let start = started else { return }
        started = nil
        renderer.onNextFrame = { isPresented, time in
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self.record(isPresented, time - start) }
            }
        }
    }

    private func record(_ isPresented: Bool, _ seconds: CFTimeInterval) {
        if !isPresented {
            rendered.append(seconds * 1000)
        } else if seconds > 0 {
            // A frame that never reached the screen (a covered window)
            // reports a zero host time.
            presented.append(seconds * 1000)
        }
        marks += 1
        guard marks == 2 else { return }
        marks = 0
        // Spaced out so each key meets an idle pipeline.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { MainActor.assumeIsolated { self.next() } }
    }

    private func finish() {
        let text = [("key to frame rendered", rendered), ("key to frame on screen", presented)].map { name, values in
            let sorted = values.sorted()
            guard !sorted.isEmpty else { return "\(name): no samples\n" }
            let at = { (q: Double) in sorted[min(sorted.count - 1, Int(Double(sorted.count) * q))] }
            return String(format: "%@: samples %d, min %.2f ms, median %.2f ms, p95 %.2f ms, max %.2f ms\n",
                          name, sorted.count, sorted[0], at(0.5), at(0.95), sorted[sorted.count - 1])
        }.joined()
        try? text.write(toFile: path, atomically: true, encoding: .utf8)
    }
}
#endif
