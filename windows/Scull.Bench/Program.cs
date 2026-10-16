using System.Globalization;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using Scull.Core;
using Scull.Render;

namespace Scull.Bench;

internal static class Program
{
    private static int Main(string[] args)
    {
        if (!OperatingSystem.IsWindowsVersionAtLeast(8, 1))
        {
            Console.Error.WriteLine("The renderer benchmark needs Windows (Direct3D 11).");
            return 1;
        }
        LocateCore();
        string report = Report(BenchInput.Fixed());
        if (args.Length > 0)
        {
            File.WriteAllText(args[0], report);
        }
        Console.Write(report);
        return 0;
    }

    /// <summary>The Markdown of a run on WARP and, where there is one, a hardware device.</summary>
    [SupportedOSPlatform("windows8.1")]
    internal static string Report(byte[] input, int singleFrames = 200)
    {
        string where = Environment.GetEnvironmentVariable("GITHUB_ACTIONS") == "true" ? "a GitHub Actions runner VM" : "a developer machine";
        var text = new System.Text.StringBuilder();
        text.AppendLine(CultureInfo.InvariantCulture,
            $"Run on {DateTime.UtcNow:yyyy-MM-dd} on {where}: {RuntimeInformation.OSDescription}, {Environment.GetEnvironmentVariable("PROCESSOR_IDENTIFIER")}, {Environment.ProcessorCount} logical processors. {input.Length} bytes in an 80 x 24 grid, drawn at {RendererBenchPixels}.");
        foreach (bool warp in new[] { true, false })
        {
            text.AppendLine().AppendLine(warp ? "### WARP (software rasteriser)" : "### Hardware adapter").AppendLine();
            RenderDevice device;
            try
            {
                device = new RenderDevice(warp);
            }
            catch (COMException e)
            {
                text.AppendLine(CultureInfo.InvariantCulture, $"No hardware device: {e.Message.Trim()}");
                continue;
            }
            using (device)
            {
                text.AppendLine(RendererBench.Header).AppendLine(RendererBench.Run(device, input, singleFrames: singleFrames));
            }
        }
        return text.ToString();
    }

    private const string RendererBenchPixels = "Consolas 28 px (14 points at a scale of 2)";

    // The release core, which cargo builds for `just windows-bench`: a debug build's numbers mean nothing.
    private static void LocateCore()
    {
        string? path = Environment.GetEnvironmentVariable("SCULL_FFI_LIB");
        for (string? dir = AppContext.BaseDirectory; path is null && dir is not null; dir = Path.GetDirectoryName(dir))
        {
            string candidate = Path.Combine(dir, "target", "release", "scull_ffi.dll");
            path = File.Exists(candidate) ? candidate : null;
        }
        nint library = NativeLibrary.Load(path ?? throw new FileNotFoundException("target/release/scull_ffi.dll: run `just windows-bench`, or set SCULL_FFI_LIB"));
        NativeLibrary.SetDllImportResolver(typeof(Terminal).Assembly, (name, _, _) => name == "scull_ffi" ? library : 0);
    }
}
