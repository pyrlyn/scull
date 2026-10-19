using Scull.Native;

namespace Scull.Core;

/// <summary>The C ABI this assembly was built against and the one the loaded library speaks.</summary>
public static class Abi
{
    /// <summary><c>major &lt;&lt; 16 | minor</c> of the bindings compiled into this assembly.</summary>
    public static uint HostVersion => NativeMethods.TT_ABI_VERSION;

    /// <summary><c>major &lt;&lt; 16 | minor</c> of the loaded library.</summary>
    public static uint LibraryVersion => NativeMethods.tt_abi_version();

    /// <summary>
    /// Throws <see cref="AbiMismatchException"/> when the loaded library speaks another ABI.
    /// A host calls it at start-up; <see cref="Terminal"/> also calls it before its first handle.
    /// </summary>
    public static void EnsureCompatible()
    {
        uint library = LibraryVersion;
        if (!Compatible(HostVersion, library))
        {
            throw new AbiMismatchException(HostVersion, library);
        }
    }

    // scull.h: a host refuses another major, and while the major is 0 every
    // minor may break, so the minor must match too. Past 0 a minor only adds,
    // so a newer library still serves an older host.
    internal static bool Compatible(uint host, uint library)
    {
        uint major = host >> 16;
        if (library >> 16 != major)
        {
            return false;
        }
        uint hostMinor = host & 0xFFFF, libraryMinor = library & 0xFFFF;
        return major == 0 ? libraryMinor == hostMinor : libraryMinor >= hostMinor;
    }
}
