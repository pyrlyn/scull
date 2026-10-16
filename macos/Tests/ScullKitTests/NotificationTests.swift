import Testing
@testable import ScullKit

@Test func controlCharactersBecomeSpacesOrVanish() {
    #expect(NotificationPolicy.clean("a\tb\r\nc", max: 50) == "a b c")
    #expect(NotificationPolicy.clean("a\u{7}\u{1b}\u{9b}b", max: 50) == "ab")
    #expect(NotificationPolicy.clean("  x \n\n", max: 50) == "x", "trimmed")
}

@Test func anInvisibleReorderingCharacterIsDropped() {
    #expect(NotificationPolicy.clean("\u{202E}exe.txt", max: 50) == "exe.txt")
    #expect(NotificationPolicy.clean("👨‍👩‍👧", max: 50) == "👨‍👩‍👧", "the joiner of an emoji stays")
}

@Test func longTextIsCutToWholeCharactersWithAnEllipsis() {
    #expect(NotificationPolicy.clean("abcdefgh", max: 5) == "abcd…")
    #expect(NotificationPolicy.clean("abcde", max: 5) == "abcde", "exactly the limit stays whole")
    // A surrogate pair, a wide character and a combining mark each count once and are never halved.
    #expect(NotificationPolicy.clean("😀世e\u{301}zz", max: 4) == "😀世e\u{301}…")
}

@Test func anEmptyBodyIsNoNotification() {
    #expect(NotificationPolicy.make(title: "Build", body: " \n\u{7} ") == nil)
    #expect(NotificationPolicy.make(title: "", body: "hi") == Notice(title: "", body: "hi"))
    #expect(NotificationPolicy.make(title: "t", body: String(repeating: "x", count: 1000))?.body.count == NotificationPolicy.bodyMax)
}

@Test func theTitleFallsBackToThePaneThenTheAppName() {
    #expect(Notice(title: "Build", body: "done").title(fallback: "vim") == "Build")
    #expect(Notice(title: "", body: "done").title(fallback: "vim") == "vim")
    #expect(Notice(title: "", body: "done").title(fallback: nil) == "Scull")
    #expect(Notice(title: "", body: "done").title(fallback: "\u{7} ") == "Scull", "a fallback of nothing printable")
}

@Test func theLimiterAllowsABurstThenOneEveryFewSeconds() {
    var limiter = NotificationLimiter()
    var now = 1000.0
    let first = (0..<3).map { _ in limiter.allow(at: now) }
    #expect(first == [true, true, true])
    let fourth = limiter.allow(at: now)
    #expect(!fourth, "the fourth at once is dropped")
    now += 4.999
    let early = limiter.allow(at: now)
    #expect(!early)
    now += 0.001
    let refilled = limiter.allow(at: now)
    #expect(refilled, "one token after 5 seconds")
    let spent = limiter.allow(at: now)
    #expect(!spent)
    now += 60
    let allowed = (0..<10).filter { _ in limiter.allow(at: now) }.count
    #expect(allowed == 3, "the bucket refills to the burst, no further")
}

@Test func aClockThatGoesBackwardsGivesNothing() {
    var limiter = NotificationLimiter()
    let burst = (0..<3).map { _ in limiter.allow(at: 10_000) }
    #expect(burst == [true, true, true])
    let back = limiter.allow(at: 1000)
    #expect(!back)
}

@MainActor
private final class FakePoster: NotificationPoster {
    var posted: [(title: String, body: String)] = []
    func post(title: String, body: String) { posted.append((title, body)) }
}

@MainActor
@Test func osc9IsABodyWithNoTitleAndOsc777CarriesBoth() throws {
    let session = try TerminalSession(cols: 20, rows: 3)
    _ = session.feed(Array("\u{1b}]9;build finished\u{7}\u{1b}]777;notify;Build;all green\u{7}".utf8))
    _ = session.drainEvents {}
    #expect(session.takeNotices() == [Notice(title: "", body: "build finished"), Notice(title: "Build", body: "all green")])
    #expect(session.takeNotices().isEmpty, "taken once")
}

@MainActor
@Test func aSessionLimitsItsNotifications() throws {
    let session = try TerminalSession(cols: 20, rows: 3)
    var now = 0.0
    session.clock = { now }
    let burst = (0..<5).map { "\u{1b}]9;n\($0)\u{7}" }.joined()
    _ = session.feed(Array(burst.utf8))
    _ = session.drainEvents {}
    #expect(session.takeNotices().count == 3, "the burst, the rest dropped")
    _ = session.feed(Array("\u{1b}]9;more\u{7}".utf8))
    _ = session.drainEvents {}
    #expect(session.takeNotices().isEmpty)
    now += 5
    _ = session.feed(Array("\u{1b}]9;later\u{7}".utf8))
    _ = session.drainEvents {}
    #expect(session.takeNotices().count == 1)
}

@MainActor
@Test func anEmptyNotificationSpendsNoToken() throws {
    let session = try TerminalSession(cols: 20, rows: 3)
    session.clock = { 0 }
    _ = session.feed(Array("\u{1b}]9;\u{7}\u{1b}]9;\u{7}\u{1b}]9;\u{7}\u{1b}]9;\u{7}\u{1b}]9;a\u{7}\u{1b}]9;b\u{7}\u{1b}]9;c\u{7}".utf8))
    _ = session.drainEvents {}
    #expect(session.takeNotices().count == 3)
}

@MainActor
@Test func nothingShowsWhileTheWindowIsTheKeyOne() {
    let poster = FakePoster()
    let notices = [Notice(title: "", body: "one"), Notice(title: "Build", body: "two")]
    NotificationPolicy.post(notices, windowIsKey: true, fallbackTitle: "vim", to: poster)
    #expect(poster.posted.isEmpty)
    NotificationPolicy.post(notices, windowIsKey: false, fallbackTitle: "vim", to: poster)
    #expect(poster.posted.map(\.title) == ["vim", "Build"])
    #expect(poster.posted.map(\.body) == ["one", "two"])
}

@MainActor
@Test func theSystemPosterIsQuietOutsideAnAppBundle() {
    // Under `swift test` there is no notification centre; posting must neither crash nor ask.
    UserNotificationPoster.shared.post(title: "t", body: "b")
}
