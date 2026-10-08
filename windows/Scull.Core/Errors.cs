using Scull.Native;

namespace Scull.Core;

/// <summary>What a core call answered; the values are <c>tt_status</c>'s.</summary>
public enum Status
{
    Ok = tt_status.TT_OK,
    Invalid = tt_status.TT_INVALID,
    AbiMismatch = tt_status.TT_ABI_MISMATCH,
    Poisoned = tt_status.TT_POISONED,
    Panic = tt_status.TT_PANIC,
    Empty = tt_status.TT_EMPTY,
    Closed = tt_status.TT_CLOSED,
    Io = tt_status.TT_IO,
    Full = tt_status.TT_FULL,
}

/// <summary>A core call that did not answer <see cref="Status.Ok"/>.</summary>
public class ScullException : Exception
{
    public ScullException(Status status, string message)
        : base(message) => Status = status;

    public Status Status { get; }

    internal static ScullException From(tt_status status, string call) => status switch
    {
        tt_status.TT_POISONED or tt_status.TT_PANIC => new TerminalPoisonedException((Status)status, call),
        tt_status.TT_ABI_MISMATCH => new AbiMismatchException(Abi.HostVersion, Abi.LibraryVersion),
        _ => new ScullException((Status)status, $"{call} answered {(Status)status}"),
    };
}

/// <summary>
/// The core hit a bug inside this terminal. Only disposing it is left; other
/// terminals keep running, so a host shows a notice in its place.
/// </summary>
public sealed class TerminalPoisonedException : ScullException
{
    internal TerminalPoisonedException(Status status, string call)
        : base(status, $"{call} answered {status}: the terminal is poisoned") { }
}

/// <summary>The loaded library speaks another ABI than the bindings this assembly was built with.</summary>
public sealed class AbiMismatchException : ScullException
{
    public AbiMismatchException(uint hostVersion, uint libraryVersion)
        : base(Status.AbiMismatch, $"built for ABI {Format(hostVersion)}, the library is {Format(libraryVersion)}")
    {
        HostVersion = hostVersion;
        LibraryVersion = libraryVersion;
    }

    public uint HostVersion { get; }

    public uint LibraryVersion { get; }

    private static string Format(uint version) => $"{version >> 16}.{version & 0xFFFF}";
}
