//! The C ABI that the Swift and C# hosts link: opaque handles, the frame
//! snapshot, the wakeup callback and the polled event queue. Every export is
//! guarded so no panic crosses the boundary.
