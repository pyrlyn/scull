using System.Runtime.InteropServices;
using Scull.Core;

[assembly: Parallelize(Scope = ExecutionScope.MethodLevel)]

namespace Scull.Core.Tests;

/// <summary>
/// Points <c>scull_ffi</c> at the library cargo built in the repository's
/// <c>target/debug</c>, or at <c>SCULL_FFI_LIB</c> when set. A resolver, not
/// a copy into the test output: a copy goes stale the moment the Rust side is
/// rebuilt, and an app ships the library beside its executable, where the
/// runtime's own search finds it with no resolver at all.
/// </summary>
[TestClass]
public static class NativeLibrarySetup
{
    [AssemblyInitialize]
    public static void Load(TestContext context)
    {
        _ = context;
        nint library = NativeLibrary.Load(Environment.GetEnvironmentVariable("SCULL_FFI_LIB") ?? RepositoryBuild());
        DllImportResolver resolve = (name, _, _) => name == "scull_ffi" ? library : 0;
        NativeLibrary.SetDllImportResolver(typeof(Terminal).Assembly, resolve);
        NativeLibrary.SetDllImportResolver(typeof(NativeLibrarySetup).Assembly, resolve);
    }

    private static string RepositoryBuild()
    {
        string file = OperatingSystem.IsWindows() ? "scull_ffi.dll"
            : OperatingSystem.IsMacOS() ? "libscull_ffi.dylib"
            : "libscull_ffi.so";
        for (var dir = new DirectoryInfo(AppContext.BaseDirectory); dir is not null; dir = dir.Parent)
        {
            if (File.Exists(Path.Combine(dir.FullName, "Cargo.lock")))
            {
                return Path.Combine(dir.FullName, "target", "debug", file);
            }
        }
        throw new FileNotFoundException($"no Cargo.lock above {AppContext.BaseDirectory}; set SCULL_FFI_LIB", file);
    }
}
