using System.Diagnostics;
using System.Globalization;
using System.Runtime.Versioning;
using Scull.Core;
using Scull.Render;

namespace Scull.Bench;

/// <summary>
/// The method of macos/Tests/ScullKitTests/RendererBenchmark.swift: the T3 input fed through the real core
/// in chunks with the frame updated and, in the second kind of row, drawn offscreen. Direct3D queues work,
/// so "queued" rows read the target back once at the end (that waits for every frame) and "waited" rows
/// read it back after every frame, which is how long a frame takes from start to finish and includes the copy.
/// </summary>
[SupportedOSPlatform("windows8.1")]
internal static class RendererBench
{
    public const string Header = "| case | chunk | frames | seconds | MiB/s | ms per frame |\n| --- | --- | --- | --- | --- | --- |";

    private const int Cols = 80, Rows = 24;

    // 14 points at the scale of 2 the macOS numbers use; Consolas ships with every Windows.
    private const string Family = "Consolas";
    private const float FontPixels = 28;

    public static string Run(RenderDevice device, byte[] input, int singleFrames = 200)
    {
        using var renderer = new Renderer(device, Family, FontPixels);
        CellMetrics m = renderer.Metrics;
        using var target = new OffscreenTarget(device, Cols * m.Width, Rows * m.Height);
        double mib = input.Length / 1_048_576.0;
        var lines = new List<string>();

        foreach (int chunk in new[] { 4096, 65536 })
        {
            foreach (bool draws in new[] { false, true })
            {
                using var terminal = new Terminal(Cols, Rows);
                Warm(terminal, renderer, target);
                int frames = 0;
                var clock = Stopwatch.StartNew();
                for (int start = 0; start < input.Length; start += chunk)
                {
                    terminal.Feed(input.AsSpan(start, Math.Min(chunk, input.Length - start)));
                    if (!terminal.Update() || !draws)
                    {
                        continue;
                    }
                    renderer.Absorb(terminal.View);
                    renderer.Render(terminal.View, target);
                    frames++;
                }
                if (draws)
                {
                    _ = target.ReadPixels();
                }
                double s = clock.Elapsed.TotalSeconds;
                lines.Add(string.Create(CultureInfo.InvariantCulture,
                    $"| {(draws ? "feed, update, render (queued)" : "feed, update")} | {chunk} | {frames} | {s:F6} | {mib / s:F2} | {(frames > 0 ? (s * 1000 / frames).ToString("F3", CultureInfo.InvariantCulture) : "-")} |"));
            }
        }

        // One frame's cost when one row changes (an echo) and when every row does (a full screen of new text).
        byte[] screen = input[..(Cols * Rows)];
        foreach ((string name, byte[] bytes) in new[] { ("one row", "x"u8.ToArray()), ("all rows", screen) })
        {
            foreach (bool waited in new[] { false, true })
            {
                using var terminal = new Terminal(Cols, Rows);
                terminal.Feed(screen);
                Warm(terminal, renderer, target);
                var clock = Stopwatch.StartNew();
                for (int i = 0; i < singleFrames; i++)
                {
                    terminal.Feed(bytes);
                    terminal.Update();
                    renderer.Absorb(terminal.View);
                    renderer.Render(terminal.View, target);
                    if (waited)
                    {
                        _ = target.ReadPixels();
                    }
                }
                if (!waited)
                {
                    _ = target.ReadPixels();
                }
                double s = clock.Elapsed.TotalSeconds;
                lines.Add(string.Create(CultureInfo.InvariantCulture,
                    $"| {name} changed ({(waited ? "waited each frame" : "queued")}) | - | {singleFrames} | {s:F6} | - | {s * 1000 / singleFrames:F3} |"));
            }
        }
        return string.Join('\n', lines);
    }

    // Warms the atlas and the shaper so the run times steady state, and drains the GPU before the clock starts.
    private static void Warm(Terminal terminal, Renderer renderer, OffscreenTarget target)
    {
        terminal.Update();
        renderer.Absorb(terminal.View);
        renderer.Render(terminal.View, target);
        _ = target.ReadPixels();
    }
}
