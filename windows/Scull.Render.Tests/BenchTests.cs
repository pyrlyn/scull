using System.Runtime.Versioning;
using Scull.Bench;
using Scull.Core;

namespace Scull.Render.Tests;

[TestClass]
public sealed class BenchTests
{
    // The same constant `scull-bench` pins in `fixed_input_matches_the_windows_bench_copy`.
    private const ulong FixedInputHash = 0xb9ffd623f56f0e6d;

    [TestMethod]
    public void TheInputIsTheT3PayloadByteForByte()
    {
        byte[] input = BenchInput.Fixed();
        Assert.HasCount(1_000_000, input);
        Assert.AreEqual(FixedInputHash, BenchInput.Hash(input));
        Assert.AreEqual((byte)'\n', input[79]);
        Assert.AreEqual((byte)'A', input[0]);
    }

    [TestMethod]
    [OSCondition(OperatingSystems.Windows)]
    [SupportedOSPlatform("windows8.1")]
    public void ASmallRunOnWarpGivesEveryRow()
    {
        using var device = new RenderDevice(warp: true);
        string[] rows = RendererBench.Run(device, BenchInput.Fixed()[..20_000], singleFrames: 3).Split('\n');

        Assert.HasCount(8, rows);
        foreach (string row in rows)
        {
            Assert.HasCount(8, row.Split('|'), row); // 6 cells between 7 bars, plus the empty ends
        }
        StringAssert.Contains(rows[0], "feed, update");

        Assert.IsFalse(rows[1].Contains("| - |", StringComparison.Ordinal), "a render row has a frame time");
    }
}
