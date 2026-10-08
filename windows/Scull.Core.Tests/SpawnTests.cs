namespace Scull.Core.Tests;

/// <summary>
/// Terminals with a real child on a real PTY, driven as a host drives them:
/// wait for the wakeup, then poll and update. The same children as
/// <c>scull-ffi/tests/spawn.rs</c>.
/// </summary>
[TestClass]
public sealed class SpawnTests
{
    // A child that is slow to start on a loaded machine still finishes well inside this.
    private static readonly TimeSpan Generous = TimeSpan.FromSeconds(30);

    [TestMethod]
    public void OutputAndTheExitCodeArriveThroughWakeups()
    {
        SpawnOptions options = OperatingSystem.IsWindows()
            ? new SpawnOptions { Program = "cmd", Arguments = ["/C", "echo hi & exit 3"] }
            : new SpawnOptions { Program = "/bin/sh", Arguments = ["-c", "printf hi; exit 3"] };
        using var host = new Host(options);

        TerminalEvent exit = host.WaitForExit();

        Assert.AreEqual(3u, exit.ExitCode);
        Assert.IsFalse(exit.Signaled);
        Assert.Contains("hi", host.Terminal.ReadText());
        Assert.IsGreaterThan(0, host.Wakeups);
    }

    [TestMethod]
    [OSCondition(OperatingSystems.Linux | OperatingSystems.OSX)]
    public void AResizeReachesTheChild()
    {
        // The trap is set before "ready", so the resize cannot beat it; the
        // shell runs it once the current sleep returns.
        var options = new SpawnOptions
        {
            Program = "/bin/sh",
            Arguments = ["-c", "trap 'stty size; exit 0' WINCH; echo ready; while :; do sleep 0.1; done"],
        };
        using var host = new Host(options);
        host.WaitFor("ready", () => host.Terminal.ReadText().Contains("ready", StringComparison.Ordinal));

        host.Terminal.ResizeBegin();
        host.Terminal.Resize(30, 5, 300, 100);

        host.WaitFor("the new size", () => host.Terminal.ReadText().Contains("5 30", StringComparison.Ordinal));
        Assert.AreEqual(0u, host.WaitForExit().ExitCode);
    }

    [TestMethod]
    public void AProgramThatIsNotThereIsAnIoError()
    {
        var options = new SpawnOptions { Program = "scull-no-such-program-" + Guid.NewGuid().ToString("N") };
        ScullException failed = Assert.Throws<ScullException>(() => Terminal.Spawn(20, 4, () => { }, options));
        Assert.AreEqual(Status.Io, failed.Status);
    }

    /// <summary>What a host sees of one spawned terminal.</summary>
    internal sealed class Host : IDisposable
    {
        private readonly SemaphoreSlim woken = new(0);
        private int wakeups;

        public Host(SpawnOptions options) =>
            Terminal = Terminal.Spawn(20, 4, () =>
            {
                Interlocked.Increment(ref wakeups);
                woken.Release();
            }, options);

        public Terminal Terminal { get; }

        public int Wakeups => Volatile.Read(ref wakeups);

        /// <summary>Drains events and updates after every wakeup until the child exits.</summary>
        public TerminalEvent WaitForExit()
        {
            TerminalEvent? exit = null;
            WaitFor("the exit", () =>
            {
                while (Terminal.TryPollEvent(out TerminalEvent next))
                {
                    if (next.Kind == EventKind.ChildExited)
                    {
                        exit = next;
                    }
                }
                return exit is not null;
            });
            return exit!.Value;
        }

        /// <summary>Looks after every wakeup, not on a clock, until <paramref name="done"/> holds.</summary>
        public void WaitFor(string what, Func<bool> done)
        {
            DateTime deadline = DateTime.UtcNow + Generous;
            while (true)
            {
                bool finished = done();
                Terminal.Update();
                if (finished)
                {
                    return;
                }
                TimeSpan left = deadline - DateTime.UtcNow;
                if (left <= TimeSpan.Zero || !woken.Wait(left))
                {
                    Assert.Fail($"timed out waiting for {what}; screen so far: {Terminal.ReadText()}");
                }
            }
        }

        // The terminal goes first, so no wakeup reaches the semaphore after it is disposed.
        public void Dispose()
        {
            Terminal.Dispose();
            woken.Dispose();
        }
    }
}
