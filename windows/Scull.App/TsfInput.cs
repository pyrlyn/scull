using System.Drawing;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using Scull.Core;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.System.Com;
using Windows.Win32.System.Variant;
using Windows.Win32.UI.TextServices;

namespace Scull.App;

/// <summary>What the terminal answers the input method, and what it does with the composition.</summary>
internal interface ITsfClient
{
    /// <summary>The window the input method's candidate list belongs to.</summary>
    nint Window { get; }

    /// <summary>The composing text's cells, in screen pixels: where the candidate window goes.</summary>
    Rectangle CompositionBox();

    /// <summary>The terminal's visible area, in screen pixels.</summary>
    Rectangle Viewport();

    /// <summary>
    /// The composing text changed; empty when there is none. <paramref name="caret"/> and the
    /// <paramref name="clauses"/> the input method marked are UTF-16 offsets into the text.
    /// </summary>
    void Preedit(string text, int caret, IReadOnlyList<PreeditClause> clauses);

    /// <summary>Text the input method finished.</summary>
    void Commit(string text);
}

/// <summary>
/// Text Services Framework for the terminal, the way Windows Terminal does it
/// (<c>src/tsf/Implementation.cpp</c> in microsoft/terminal): the terminal is the owner of a
/// TSF context but keeps no text store, because it has no document to answer for. The
/// input method composes inside the context; this reads the composition back in an edit
/// session and splits it with <see cref="CompositionSplit"/>. The three sinks the context
/// calls and the edit session are one native block with a vtable each, built by hand as in
/// <c>Scull.Render/TextAnalysis.cs</c>; the block is freed in <see cref="Dispose"/>, after
/// TSF has let go, and reference counts only count. Every call happens on the UI thread.
/// </summary>
internal sealed unsafe class TsfInput : IDisposable
{
    // TF_TMAE_CONSOLE makes the context behave like a console's: no interim characters for
    // Korean, which would send half-finished syllables to the shell. The tip is not activated
    // at once and the keyboard layout is left alone, as in conhost.
    private const uint ActivationFlags = PInvoke.TF_TMAE_NOACTIVATETIP | PInvoke.TF_TMAE_NOACTIVATEKEYBOARDLAYOUT | PInvoke.TF_TMAE_CONSOLE;

    // TS_SS_TRANSITORY: finished text is not expected to stay in the document, which is also
    // what makes TSF treat this like a legacy IME client that works. TS_SS_NOHIDDENTEXT: no markup.
    private const uint StaticFlags = PInvoke.TS_SS_TRANSITORY | PInvoke.TS_SS_NOHIDDENTEXT;

    private static readonly HRESULT Ok = (HRESULT)0;
    private static readonly HRESULT NotImplemented = (HRESULT)unchecked((int)0x80004001);
    private static readonly HRESULT NoInterface = (HRESULT)unchecked((int)0x80004002);
    private static readonly Guid Unknown = new(0x00000000, 0x0000, 0x0000, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46);

    // Slots are the COM objects the block holds, in the order of their vtable pointers.
    private const int OwnerSlot = 0, CompositionSlot = 1, EditSinkSlot = 2, SessionSlot = 3;

    // A COM object: the vtable pointer first, then a way back to the block.
    [StructLayout(LayoutKind.Sequential)]
    private struct Slot
    {
        public void* Vtable;
        public Block* Block;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct Block
    {
        public Slot Owner;
        public Slot Composition;
        public Slot EditSink;
        public Slot Session;
        public nint Self;
        public uint References;
        public ITfContextOwner.Vtbl OwnerTable;
        public ITfContextOwnerCompositionSink.Vtbl CompositionTable;
        public ITfTextEditSink.Vtbl EditSinkTable;
        public ITfEditSession.Vtbl SessionTable;
    }

    private readonly ITsfClient client;
    private readonly CompositionTracker tracker = new();
    private readonly Block* block;
    private GCHandle self;
    private ITfThreadMgrEx* threadMgr;
    private ITfDocumentMgr* documentMgr;
    private ITfContext* context;
    private ITfSource* source;
    // Made on the first composition that marks its text, and kept: a lookup per key would be a call into COM for each.
    private ITfCategoryMgr* categories;
    private ITfDisplayAttributeMgr* displays;
    private uint clientId, ownerCookie = PInvoke.TF_INVALID_COOKIE, editCookie = PInvoke.TF_INVALID_COOKIE;
    private bool requested, associated;

    public TsfInput(ITsfClient client)
    {
        this.client = client;
        block = (Block*)NativeMemory.AllocZeroed((nuint)sizeof(Block));
        self = GCHandle.Alloc(this, GCHandleType.Weak);
        Build();
        try
        {
            Activate();
        }
        catch
        {
            Dispose();
            throw;
        }
    }

    /// <summary>A composition is open: the keys it takes and the characters it makes are not the terminal's.</summary>
    public bool Composing => tracker.Active;

    private void Build()
    {
        block->Self = GCHandle.ToIntPtr(self);
        block->Owner = new Slot { Vtable = &block->OwnerTable, Block = block };
        block->Composition = new Slot { Vtable = &block->CompositionTable, Block = block };
        block->EditSink = new Slot { Vtable = &block->EditSinkTable, Block = block };
        block->Session = new Slot { Vtable = &block->SessionTable, Block = block };

        block->OwnerTable.QueryInterface_1 = (delegate* unmanaged[Stdcall]<ITfContextOwner*, Guid*, void**, HRESULT>)&QueryInterface;
        block->OwnerTable.AddRef_2 = (delegate* unmanaged[Stdcall]<ITfContextOwner*, uint>)&AddRef;
        block->OwnerTable.Release_3 = (delegate* unmanaged[Stdcall]<ITfContextOwner*, uint>)&Release;
        block->OwnerTable.GetACPFromPoint_4 = &GetAcpFromPoint;
        block->OwnerTable.GetTextExt_5 = &GetTextExt;
        block->OwnerTable.GetScreenExt_6 = &GetScreenExt;
        block->OwnerTable.GetStatus_7 = &GetStatus;
        block->OwnerTable.GetWnd_8 = &GetWnd;
        block->OwnerTable.GetAttribute_9 = &GetAttribute;

        block->CompositionTable.QueryInterface_1 = (delegate* unmanaged[Stdcall]<ITfContextOwnerCompositionSink*, Guid*, void**, HRESULT>)&QueryInterface;
        block->CompositionTable.AddRef_2 = (delegate* unmanaged[Stdcall]<ITfContextOwnerCompositionSink*, uint>)&AddRef;
        block->CompositionTable.Release_3 = (delegate* unmanaged[Stdcall]<ITfContextOwnerCompositionSink*, uint>)&Release;
        block->CompositionTable.OnStartComposition_4 = &OnStartComposition;
        block->CompositionTable.OnUpdateComposition_5 = &OnUpdateComposition;
        block->CompositionTable.OnEndComposition_6 = &OnEndComposition;

        block->EditSinkTable.QueryInterface_1 = (delegate* unmanaged[Stdcall]<ITfTextEditSink*, Guid*, void**, HRESULT>)&QueryInterface;
        block->EditSinkTable.AddRef_2 = (delegate* unmanaged[Stdcall]<ITfTextEditSink*, uint>)&AddRef;
        block->EditSinkTable.Release_3 = (delegate* unmanaged[Stdcall]<ITfTextEditSink*, uint>)&Release;
        block->EditSinkTable.OnEndEdit_4 = &OnEndEdit;

        block->SessionTable.QueryInterface_1 = (delegate* unmanaged[Stdcall]<ITfEditSession*, Guid*, void**, HRESULT>)&QueryInterface;
        block->SessionTable.AddRef_2 = (delegate* unmanaged[Stdcall]<ITfEditSession*, uint>)&AddRef;
        block->SessionTable.Release_3 = (delegate* unmanaged[Stdcall]<ITfEditSession*, uint>)&Release;
        block->SessionTable.DoEditSession_4 = &DoEditSession;
    }

    private void Activate()
    {
        Guid clsid = PInvoke.CLSID_TF_ThreadMgr;
        ITfThreadMgrEx* manager;
        PInvoke.CoCreateInstance(clsid, null, CLSCTX.CLSCTX_INPROC_SERVER, out manager).ThrowOnFailure();
        threadMgr = manager;

        uint id;
        threadMgr->ActivateEx(&id, ActivationFlags);
        clientId = id;
        ITfDocumentMgr* document;
        threadMgr->CreateDocumentMgr(&document);
        documentMgr = document;

        ITfContext* created;
        uint textStoreCookie;
        documentMgr->CreateContext(clientId, 0, (IUnknown*)&block->Composition, &created, &textStoreCookie);
        context = created;

        ITfSource* sources;
        context->QueryInterface(out sources).ThrowOnFailure();
        source = sources;
        Guid ownerId = ITfContextOwner.IID_Guid, editId = ITfTextEditSink.IID_Guid;
        uint cookie;
        source->AdviseSink(&ownerId, (IUnknown*)&block->Owner, &cookie);
        ownerCookie = cookie;
        source->AdviseSink(&editId, (IUnknown*)&block->EditSink, &cookie);
        editCookie = cookie;

        documentMgr->Push(context);
    }

    /// <summary>The terminal has the keyboard: TSF sends composition to this context.</summary>
    public void Focus()
    {
        if (!associated)
        {
            ITfDocumentMgr* previous;
            threadMgr->AssociateFocus((HWND)client.Window, documentMgr, &previous);
            Release(previous);
            associated = true;
        }
        threadMgr->SetFocus(documentMgr);
    }

    /// <summary>The terminal lost the keyboard: a composition left open is ended, and its text is shown no more.</summary>
    public void Unfocus()
    {
        if (!tracker.Active && !tracker.Pending)
        {
            return;
        }
        tracker.Reset();
        client.Preedit("", 0, []);
        ITfContextOwnerCompositionServices* services;
        if (context->QueryInterface(out services).Succeeded)
        {
            services->TerminateComposition(null);
            Release(services);
        }
    }

    /// <summary>
    /// Reads a composition that ended but whose text has not been read yet. A key that ended it,
    /// such as Enter, reaches the terminal at once while TSF hands over the text a moment later,
    /// so it is read here, first, before that key goes out.
    /// </summary>
    public void Flush()
    {
        if (!tracker.Pending || tracker.Active || context is null)
        {
            return;
        }
        HRESULT session;
        context->RequestEditSession(clientId, (ITfEditSession*)&block->Session, TF_CONTEXT_EDIT_CONTEXT_FLAGS.TF_ES_READWRITE | TF_CONTEXT_EDIT_CONTEXT_FLAGS.TF_ES_SYNC, &session);
    }

    // The session may only be asked for once the sink call that allows it has returned, hence async.
    private void Request()
    {
        if (requested || context is null)
        {
            return;
        }
        HRESULT session;
        context->RequestEditSession(clientId, (ITfEditSession*)&block->Session, TF_CONTEXT_EDIT_CONTEXT_FLAGS.TF_ES_READWRITE | TF_CONTEXT_EDIT_CONTEXT_FLAGS.TF_ES_ASYNC, &session);
        requested = session.Succeeded;
    }

    // Everything is derived from the context as it is now, so a late or repeated session is harmless.
    private void ReadComposition(uint ec)
    {
        requested = false;
        tracker.Read();

        ITfRange* whole;
        context->GetStart(ec, &whole);
        int length;
        whole->ShiftEnd(ec, int.MaxValue, &length, null);

        var runs = new List<(string, bool)>();
        Guid composingId = PInvoke.GUID_PROP_COMPOSING;
        ITfProperty* property;
        context->GetProperty(&composingId, &property);
        IEnumTfRanges* ranges;
        property->EnumRanges(ec, &ranges, whole);
        var batch = stackalloc ITfRange*[8];
        uint fetched;
        do
        {
            ranges->Next(8, batch, &fetched);
            for (uint i = 0; i < fetched; i++)
            {
                VARIANT value;
                property->GetValue(ec, batch[i], &value);
                bool composing = value.vt == VARENUM.VT_I4 && value.lVal != 0;
                runs.Add((TextOf(ec, batch[i]), composing));
                Release(batch[i]);
            }
        }
        while (fetched == 8);

        List<(int Start, int End, int Attribute)> marked = ReadAttributes(ec, whole);
        int caret = Caret(ec);
        CompositionSplit split = CompositionSplit.Of(runs, caret);
        if (split.Finished.Length > 0)
        {
            // Finished text leaves the context, so it is not read again as the start of the next composition.
            ITfRange* done;
            context->GetStart(ec, &done);
            int removed;
            done->ShiftEnd(ec, split.Finished.Length, &removed, null);
            done->SetText(ec, 0, null, 0);
            Release(done);
        }
        Release(ranges);
        Release(property);
        Release(whole);

        client.Preedit(split.Active, split.Caret, split.Clauses(marked));
        if (split.Finished.Length > 0)
        {
            client.Commit(split.Finished);
        }
    }

    // Where the input method marked the text as input, converted or being converted, as (start, end,
    // TF_ATTR_*) in characters from the document's start. A lookup that fails marks nothing: the text is
    // then shown as one plain clause, which is also what an input method that marks nothing gets.
    private List<(int Start, int End, int Attribute)> ReadAttributes(uint ec, ITfRange* whole)
    {
        List<(int, int, int)> found = [];
        Guid attributeId = PInvoke.GUID_PROP_ATTRIBUTE;
        ITfProperty* property;
        if (context->GetProperty(&attributeId, &property).Failed)
        {
            return found;
        }
        IEnumTfRanges* ranges;
        if (property->EnumRanges(ec, &ranges, whole).Succeeded)
        {
            var batch = stackalloc ITfRange*[8];
            uint fetched;
            do
            {
                ranges->Next(8, batch, &fetched);
                for (uint i = 0; i < fetched; i++)
                {
                    VARIANT value;
                    if (property->GetValue(ec, batch[i], &value).Succeeded && value.vt == VARENUM.VT_I4 && DisplayAttribute((uint)value.lVal) is int attribute)
                    {
                        found.Add((Position(ec, batch[i], TfAnchor.TF_ANCHOR_START), Position(ec, batch[i], TfAnchor.TF_ANCHOR_END), attribute));
                    }
                    Release(batch[i]);
                }
            }
            while (fetched == 8);
            Release(ranges);
        }
        Release(property);
        return found;
    }

    // TF_DISPLAYATTRIBUTE.bAttr of the attribute a guid atom stands for; null when TSF cannot say.
    private int? DisplayAttribute(uint atom)
    {
        if (categories is null)
        {
            Guid categoryClass = PInvoke.CLSID_TF_CategoryMgr;
            if (PInvoke.CoCreateInstance(categoryClass, null, CLSCTX.CLSCTX_INPROC_SERVER, out ITfCategoryMgr* made).Failed)
            {
                return null;
            }
            categories = made;
        }
        if (displays is null)
        {
            Guid displayClass = PInvoke.CLSID_TF_DisplayAttributeMgr;
            if (PInvoke.CoCreateInstance(displayClass, null, CLSCTX.CLSCTX_INPROC_SERVER, out ITfDisplayAttributeMgr* made).Failed)
            {
                return null;
            }
            displays = made;
        }
        Guid id;
        if (categories->GetGUID(atom, &id).Failed)
        {
            return null;
        }
        ITfDisplayAttributeInfo* info;
        Guid owner;
        if (displays->GetDisplayAttributeInfo(&id, &info, &owner).Failed || info is null)
        {
            return null;
        }
        TF_DISPLAYATTRIBUTE attribute;
        int? kind = info->GetAttributeInfo(&attribute).Succeeded ? (int)attribute.bAttr : null;
        Release(info);
        return kind;
    }

    // The characters from the start of the document to one end of a range.
    private int Position(uint ec, ITfRange* target, TfAnchor anchor)
    {
        ITfRange* start;
        context->GetStart(ec, &start);
        var halt = new TF_HALTCOND { pHaltRange = target, aHaltPos = anchor };
        int position;
        start->ShiftEnd(ec, int.MaxValue, &position, &halt);
        Release(start);
        return position;
    }

    // The caret as a count of characters from the start of the document.
    private int Caret(uint ec)
    {
        TF_SELECTION selection;
        uint count;
        if (context->GetSelection(ec, PInvoke.TF_DEFAULT_SELECTION, 1, &selection, &count).Failed || count != 1)
        {
            return int.MaxValue;
        }
        ITfRange* start;
        context->GetStart(ec, &start);
        var halt = new TF_HALTCOND
        {
            pHaltRange = selection.range,
            aHaltPos = selection.style.ase == TfActiveSelEnd.TF_AE_START ? TfAnchor.TF_ANCHOR_START : TfAnchor.TF_ANCHOR_END,
        };
        int position;
        start->ShiftEnd(ec, int.MaxValue, &position, &halt);
        Release(start);
        Release(selection.range);
        return position;
    }

    private static string TextOf(uint ec, ITfRange* range)
    {
        var text = new System.Text.StringBuilder();
        const int capacity = 128;
        char* buffer = stackalloc char[capacity];
        uint read;
        do
        {
            range->GetText(ec, PInvoke.TF_TF_MOVESTART, buffer, capacity, &read);
            text.Append(buffer, (int)read);
        }
        while (read == capacity);
        return text.ToString();
    }

    private static void Release<T>(T* pointer)
        where T : unmanaged
    {
        if (pointer is not null)
        {
            ((IUnknown*)pointer)->Release();
        }
    }

    public void Dispose()
    {
        if (source is not null)
        {
            if (editCookie != PInvoke.TF_INVALID_COOKIE)
            {
                source->UnadviseSink(editCookie);
            }
            if (ownerCookie != PInvoke.TF_INVALID_COOKIE)
            {
                source->UnadviseSink(ownerCookie);
            }
        }
        if (associated)
        {
            ITfDocumentMgr* previous;
            threadMgr->AssociateFocus((HWND)client.Window, null, &previous);
            Release(previous);
        }
        if (documentMgr is not null)
        {
            documentMgr->Pop(PInvoke.TF_POPF_ALL);
        }
        if (threadMgr is not null)
        {
            threadMgr->Deactivate();
        }
        Release(displays);
        Release(categories);
        Release(source);
        Release(context);
        Release(documentMgr);
        Release(threadMgr);
        displays = null;
        categories = null;
        source = null;
        context = null;
        documentMgr = null;
        threadMgr = null;
        if (self.IsAllocated)
        {
            self.Free();
        }
        NativeMemory.Free(block);
    }

    private static TsfInput Of(void* pointer) =>
        (TsfInput)GCHandle.FromIntPtr(((Slot*)pointer)->Block->Self).Target!;

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT QueryInterface(void* pThis, Guid* riid, void** result)
    {
        Block* owner = ((Slot*)pThis)->Block;
        Slot* found = null;
        if (*riid == Unknown || *riid == ITfContextOwner.IID_Guid)
        {
            found = &owner->Owner;
        }
        else if (*riid == ITfContextOwnerCompositionSink.IID_Guid)
        {
            found = &owner->Composition;
        }
        else if (*riid == ITfTextEditSink.IID_Guid)
        {
            found = &owner->EditSink;
        }
        else if (*riid == ITfEditSession.IID_Guid)
        {
            found = &owner->Session;
        }
        *result = found;
        if (found is null)
        {
            return NoInterface;
        }
        Interlocked.Increment(ref owner->References);
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static uint AddRef(void* pThis) => Interlocked.Increment(ref ((Slot*)pThis)->Block->References);

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static uint Release(void* pThis) => Interlocked.Decrement(ref ((Slot*)pThis)->Block->References);

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT GetAcpFromPoint(ITfContextOwner* pThis, Point* point, uint flags, int* position) => NotImplemented;

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT GetTextExt(ITfContextOwner* pThis, int start, int end, RECT* rectangle, BOOL* clipped)
    {
        // The candidate window goes under the composing text whatever range is asked for.
        *rectangle = Native(Of(pThis).client.CompositionBox());
        *clipped = false;
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT GetScreenExt(ITfContextOwner* pThis, RECT* rectangle)
    {
        *rectangle = Native(Of(pThis).client.Viewport());
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT GetStatus(ITfContextOwner* pThis, TS_STATUS* status)
    {
        status->dwDynamicFlags = 0;
        status->dwStaticFlags = StaticFlags;
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT GetWnd(ITfContextOwner* pThis, HWND* window)
    {
        *window = (HWND)Of(pThis).client.Window;
        return Ok;
    }

    // No attribute is supported; an empty variant says so.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT GetAttribute(ITfContextOwner* pThis, Guid* attribute, VARIANT* value)
    {
        *value = default;
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT OnStartComposition(ITfContextOwnerCompositionSink* pThis, ITfCompositionView* view, BOOL* accept)
    {
        Of(pThis).tracker.Started();
        *accept = true;
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT OnUpdateComposition(ITfContextOwnerCompositionSink* pThis, ITfCompositionView* view, ITfRange* range) => Ok;

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT OnEndComposition(ITfContextOwnerCompositionSink* pThis, ITfCompositionView* view)
    {
        TsfInput input = Of(pThis);
        if (!input.tracker.Ended())
        {
            return Ok;
        }
        input.Request();
        return Ok;
    }

    // While one composition is open every edit of it is a change of the text on show.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT OnEndEdit(ITfTextEditSink* pThis, ITfContext* context, uint ecReadOnly, ITfEditRecord* record)
    {
        TsfInput input = Of(pThis);
        if (input.tracker.OnlyOne)
        {
            input.Request();
        }
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT DoEditSession(ITfEditSession* pThis, uint ec)
    {
        try
        {
            Of(pThis).ReadComposition(ec);
            return Ok;
        }
        catch (Exception exception)
        {
            // An exception must not unwind into TSF; the next edit reads the context afresh.
            return (HRESULT)exception.HResult;
        }
    }

    private static RECT Native(Rectangle box) => new() { left = box.Left, top = box.Top, right = box.Right, bottom = box.Bottom };
}
