// Desktop notifications a program asks for with OSC 9 and OSC 777. The policy
// (what text, how often, when) is plain code; only `UserNotificationPoster`
// touches the system. `Scull.Core/Notifications.cs` applies the same numbers.

import Foundation
import UserNotifications

/// A notification a program asked for, cleaned. `title` is empty when it gave none.
public struct Notice: Equatable, Sendable {
    public var title: String
    public var body: String

    public init(title: String, body: String) {
        (self.title, self.body) = (title, body)
    }

    /// The title shown: the program's, else the fallback (the pane's title), else the app's name.
    public func title(fallback: String?) -> String {
        if !title.isEmpty { return title }
        let named = fallback.map { NotificationPolicy.clean($0, max: NotificationPolicy.titleMax) } ?? ""
        return named.isEmpty ? "Scull" : named
    }
}

public enum NotificationPolicy {
    public static let titleMax = 64
    public static let bodyMax = 240

    /// A notification, or nil when the body is empty: a title alone says nothing.
    public static func make(title: String, body: String) -> Notice? {
        let text = clean(body, max: bodyMax)
        return text.isEmpty ? nil : Notice(title: clean(title, max: titleMax), body: text)
    }

    /// `text` with tabs and line ends as spaces (runs of spaces as one), other control
    /// and invisible formatting characters dropped (a right-to-left override could
    /// reorder what the reader sees), trimmed, and cut to `max` whole characters
    /// with an ellipsis.
    public static func clean(_ text: String, max: Int) -> String {
        var kept = String.UnicodeScalarView()
        for scalar in text.unicodeScalars {
            switch scalar {
            case "\t", "\n", "\r", " ":
                if let last = kept.last, last != " " { kept.append(" ") }
            default:
                if !isDropped(scalar) { kept.append(scalar) }
            }
        }
        var clean = String(kept)
        while clean.last == " " { clean.removeLast() }
        return clean.count <= max ? clean : String(clean.prefix(max - 1)) + "…"
    }

    // Formatting characters, except the joiners emoji and some scripts need.
    private static func isDropped(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.properties.generalCategory {
        case .control: true
        case .format: scalar != "\u{200C}" && scalar != "\u{200D}"
        default: false
        }
    }

    /// Posts `notices` unless the window is the one the user is in. Any pane of
    /// a window that is not key counts, so a pop-up never covers what is being read.
    @MainActor
    public static func post(_ notices: [Notice], windowIsKey: Bool, fallbackTitle: String?, to poster: some NotificationPoster) {
        guard !windowIsKey else { return }
        for notice in notices {
            poster.post(title: notice.title(fallback: fallbackTitle), body: notice.body)
        }
    }
}

/// A pane's rate limit: a burst of `burst`, then one more every `refill` seconds;
/// a notification over the limit is dropped, so a program cannot flood the desktop.
public struct NotificationLimiter {
    public static let burst = 3.0
    public static let refill = 5.0

    private var tokens = burst
    private var last: Double?

    public init() {}

    /// True when a notification may show at `now` (seconds on a monotonic clock).
    public mutating func allow(at now: Double) -> Bool {
        if let last { tokens = min(Self.burst, tokens + max(0, now - last) / Self.refill) }
        last = now
        guard tokens >= 1 else { return false }
        tokens -= 1
        return true
    }
}

@MainActor
public protocol NotificationPoster {
    func post(title: String, body: String)
}

/// The system's notification centre. Authorization is asked on the first
/// notification, and a refusal keeps it quiet. A process that is not an app
/// bundle (`swift test`, a command-line run) has no notification centre, so it
/// posts nothing.
@MainActor
public final class UserNotificationPoster: NotificationPoster {
    public static let shared = UserNotificationPoster()

    /// nil until the user was asked.
    private var allowed: Bool?
    private let presenter = Presenter()

    private static var isAppBundle: Bool { Bundle.main.bundlePath.hasSuffix(".app") }

    public func post(title: String, body: String) {
        guard Self.isAppBundle, allowed != false else { return }
        Task { await deliver(title: title, body: body) }
    }

    private func deliver(title: String, body: String) async {
        let center = UNUserNotificationCenter.current()
        center.delegate = presenter
        if allowed == nil {
            allowed = (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
        }
        guard allowed == true else { return }
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        // A refused or failed request loses one notification, nothing more.
        try? await center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
    }

    /// Shows banners while the app is active too: a window other than the
    /// terminal's (settings) can be the key one.
    private final class Presenter: NSObject, UNUserNotificationCenterDelegate {
        func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                    withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
            completionHandler([.banner, .sound])
        }
    }
}
