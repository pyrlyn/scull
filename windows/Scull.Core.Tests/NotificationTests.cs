using System.Text;

namespace Scull.Core.Tests;

[TestClass]
public sealed class NotificationTests
{
    [TestMethod]
    public void ControlCharactersBecomeSpacesOrVanish()
    {
        Assert.AreEqual("a b c", NotificationPolicy.Clean("a\tb\r\nc", 50));
        Assert.AreEqual("ab", NotificationPolicy.Clean("a\u0007\u001b\u009bb", 50));
        Assert.AreEqual("x", NotificationPolicy.Clean("  x \n\n", 50), "trimmed");
    }

    [TestMethod]
    public void AnInvisibleReorderingCharacterIsDropped()
    {
        Assert.AreEqual("exe.txt", NotificationPolicy.Clean("‮exe.txt", 50));
        Assert.AreEqual("👨‍👩‍👧", NotificationPolicy.Clean("👨‍👩‍👧", 50), "the joiner of an emoji stays");
    }

    [TestMethod]
    public void TagCharactersOutsideTheFirstPlaneAreDropped()
    {
        // U+E0041 is a tag "A": invisible, and a way to carry text no reader sees.
        Assert.AreEqual("ab", NotificationPolicy.Clean("a\U000E0041b", 50));
        Assert.AreEqual("ab", NotificationPolicy.Clean("a\U000E007Fb", 50));
        Assert.AreEqual("😀", NotificationPolicy.Clean("😀", 50), "an ordinary supplementary character stays");
    }

    [TestMethod]
    public void MarkupIsKeptAsTextAndTheCodePointsXmlForbidsAreDropped()
    {
        // The builder escapes XML (AppNotificationBuilder.AddText); the policy must not mangle or double it.
        Assert.AreEqual("</text><b>&amp;\"'", NotificationPolicy.Clean("</text><b>&amp;\"'", 50));
        // U+FFFE and U+FFFF cannot be in a toast's XML at all.
        Assert.AreEqual("ab", NotificationPolicy.Clean("a\uFFFEb\uFFFF", 50));
    }

    [TestMethod]
    public void OnlyTheSpaceThePolicyAddsIsTrimmed()
    {
        // The macOS side trims only U+0020; a no-break space is text.
        Assert.AreEqual("a\u00A0", NotificationPolicy.Clean("a\u00A0", 50));
        Assert.AreEqual("a\u00A0b", NotificationPolicy.Clean(" a\u00A0b \n", 50));
    }

    [TestMethod]
    public void LongTextIsCutToWholeCharactersWithAnEllipsis()
    {
        Assert.AreEqual("abcd…", NotificationPolicy.Clean("abcdefgh", 5));
        Assert.AreEqual("abcde", NotificationPolicy.Clean("abcde", 5), "exactly the limit stays whole");
        // A surrogate pair, a wide character and a combining mark each count once and are never halved.
        Assert.AreEqual("😀世é…", NotificationPolicy.Clean("😀世ézz", 4));
    }

    [TestMethod]
    public void AnEmptyBodyIsNoNotification()
    {
        Assert.IsNull(NotificationPolicy.Make("Build", " \n\u0007 "));
        Assert.AreEqual(new Notice("", "hi"), NotificationPolicy.Make("", "hi"));
        Assert.AreEqual(NotificationPolicy.BodyMax, NotificationPolicy.Make("t", new string('x', 1000))!.Value.Body.Length);
    }

    [TestMethod]
    public void TheTitleFallsBackToTheTabThenTheAppName()
    {
        Assert.AreEqual("Build", new Notice("Build", "done").TitleOr("vim"));
        Assert.AreEqual("vim", new Notice("", "done").TitleOr("vim"));
        Assert.AreEqual("Scull", new Notice("", "done").TitleOr(null));
        Assert.AreEqual("Scull", new Notice("", "done").TitleOr("\u0007 "), "a fallback of nothing printable");
    }

    [TestMethod]
    public void TheLimiterAllowsABurstThenOneEveryFewSeconds()
    {
        long now = 1_000;
        var limiter = new NotificationLimiter(() => now);
        Assert.IsTrue(limiter.Allow());
        Assert.IsTrue(limiter.Allow());
        Assert.IsTrue(limiter.Allow());
        Assert.IsFalse(limiter.Allow(), "the fourth at once is dropped");

        now += 4_999;
        Assert.IsFalse(limiter.Allow());
        now += 1;
        Assert.IsTrue(limiter.Allow(), "one token after 5 seconds");
        Assert.IsFalse(limiter.Allow());

        now += 60_000;
        Assert.AreEqual(3, Enumerable.Range(0, 10).Count(_ => limiter.Allow()), "the bucket refills to the burst, no further");
    }

    [TestMethod]
    public void AClockThatGoesBackwardsGivesNothing()
    {
        long now = 10_000;
        var limiter = new NotificationLimiter(() => now);
        for (int i = 0; i < 3; i++)
        {
            Assert.IsTrue(limiter.Allow());
        }
        now = 1_000;
        Assert.IsFalse(limiter.Allow());
    }

    private static DrainedEvents Drain(PaneEvents events, string output, Terminal? terminal = null)
    {
        using var own = terminal is null ? new Terminal(20, 4) : null;
        Terminal t = terminal ?? own!;
        t.Feed(Encoding.UTF8.GetBytes(output));
        return events.Drain(t);
    }

    [TestMethod]
    public void Osc9IsABodyWithNoTitle()
    {
        DrainedEvents drained = Drain(new PaneEvents(), "\e]9;build finished\a");
        Assert.AreEqual(new Notice("", "build finished"), drained.Notices.Single());
    }

    [TestMethod]
    public void Osc777CarriesATitleAndABody()
    {
        DrainedEvents drained = Drain(new PaneEvents(), "\e]777;notify;Build;all green\a");
        Assert.AreEqual(new Notice("Build", "all green"), drained.Notices.Single());
    }

    [TestMethod]
    public void ANotificationIsCleanedAndLimitedPerPane()
    {
        long now = 0;
        var events = new PaneEvents(() => now);
        using var terminal = new Terminal(20, 4);
        string burst = string.Concat(Enumerable.Range(0, 5).Select(i => $"\e]9;n{i}\a"));
        DrainedEvents first = Drain(events, burst, terminal);
        Assert.AreEqual(3, first.Notices.Count, "the burst, the rest dropped");
        Assert.AreEqual(0, Drain(events, "\e]9;more\a", terminal).Notices.Count);

        now += 5_000;
        Assert.AreEqual(1, Drain(events, "\e]9;later\a", terminal).Notices.Count);

        Assert.AreEqual(3, Drain(new PaneEvents(() => now), burst).Notices.Count, "another pane has its own bucket");
    }

    [TestMethod]
    public void NotificationsNobodyWillSeeSpendNoToken()
    {
        var events = new PaneEvents(() => 0);
        using var terminal = new Terminal(20, 4);
        terminal.Feed(Encoding.UTF8.GetBytes(string.Concat(Enumerable.Range(0, 10).Select(i => $"\e]9;hidden{i}\a"))));
        Assert.AreEqual(0, events.Drain(terminal, notify: false).Notices.Count, "the user is looking at this pane");
        Assert.AreEqual(3, Drain(events, "\e]9;a\a\e]9;b\a\e]9;c\a\e]9;d\a", terminal).Notices.Count, "the whole burst is still there");
    }

    [TestMethod]
    public void AnEmptyOsc9IsNothingAndSpendsNoToken()
    {
        var events = new PaneEvents(() => 0);
        using var terminal = new Terminal(20, 4);
        Assert.AreEqual(0, Drain(events, "\e]9;\a\e]9;\a\e]9;\a\e]9;\a", terminal).Notices.Count);
        Assert.AreEqual(3, Drain(events, "\e]9;a\a\e]9;b\a\e]9;c\a", terminal).Notices.Count);
    }
}
