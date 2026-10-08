using System.Runtime.InteropServices;

namespace Scull.Core;

/// <summary>
/// Owns one core handle. SafeHandle frees it exactly once: on the first
/// Dispose, or from its finalizer when nobody disposed it, and never while a
/// <see cref="Lease"/> still holds it, so a call in flight cannot race the free.
/// </summary>
internal sealed unsafe class NativeHandle : SafeHandle
{
    private readonly delegate*<nint, void> free;
    private GCHandle userdata;

    /// <param name="userdata">
    /// What the core's callbacks for this handle reach; freed only after the
    /// handle, because the free is what guarantees no callback is still running.
    /// </param>
    internal NativeHandle(nint pointer, delegate*<nint, void> free, GCHandle userdata = default)
        : base(0, ownsHandle: true)
    {
        this.free = free;
        this.userdata = userdata;
        SetHandle(pointer);
    }

    public override bool IsInvalid => handle == 0;

    protected override bool ReleaseHandle()
    {
        free(handle);
        if (userdata.IsAllocated)
        {
            userdata.Free();
        }
        return true;
    }
}

/// <summary>
/// The raw pointer of a <see cref="NativeHandle"/> for one call. Throws
/// <see cref="ObjectDisposedException"/> once the handle is closed.
/// </summary>
internal readonly ref struct Lease
{
    private readonly SafeHandle owner;

    internal Lease(SafeHandle owner)
    {
        bool added = false;
        owner.DangerousAddRef(ref added);
        this.owner = owner;
        Pointer = owner.DangerousGetHandle();
    }

    internal nint Pointer { get; }

    public void Dispose() => owner.DangerousRelease();
}
