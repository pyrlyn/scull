using Scull.Core;

namespace Scull.Core.Tests;

// The Rust side has the marks and their trimming; this is the managed wrapper over the real library.
[TestClass]
public sealed class PromptTests
{
    private static char TopRow(Terminal terminal)
    {
        terminal.Update();
        return (char)terminal.View.Cells[0].CodePoint;
    }

    [TestMethod]
    public void TheViewportJumpsBetweenMarkedPromptsAndStopsAtTheEnds()
    {
        using var terminal = new Terminal(10, 2);
        terminal.Feed("\x1b]133;A\aant\r\n\x1b]133;A\abee\r\n\x1b]133;A\acat\r\n\x1b]133;A\adog"u8);
        Assert.AreEqual('c', TopRow(terminal));

        Assert.IsTrue(terminal.ScrollToPrompt(forward: false));
        Assert.AreEqual('b', TopRow(terminal));
        Assert.IsTrue(terminal.ScrollToPrompt(forward: false));
        Assert.AreEqual('a', TopRow(terminal));
        Assert.IsFalse(terminal.ScrollToPrompt(forward: false), "nothing above the first prompt");
        Assert.AreEqual('a', TopRow(terminal));

        Assert.IsTrue(terminal.ScrollToPrompt(forward: true));
        Assert.IsTrue(terminal.ScrollToPrompt(forward: true));
        Assert.AreEqual('c', TopRow(terminal), "the prompt on the live screen brings the viewport home");
        Assert.IsFalse(terminal.ScrollToPrompt(forward: true));
    }

    [TestMethod]
    public void WithoutMarksThereIsNowhereToJump()
    {
        using var terminal = new Terminal(10, 2);
        terminal.Feed("one\r\ntwo\r\nthree"u8);
        Assert.IsFalse(terminal.ScrollToPrompt(forward: false));
        Assert.IsFalse(terminal.ScrollToPrompt(forward: true));
    }
}
