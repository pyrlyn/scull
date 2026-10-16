using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using Windows.Win32.Foundation;
using Windows.Win32.Graphics.DirectWrite;

namespace Scull.Render;

/// <summary>
/// The text DirectWrite's script analysis and font fallback read, and the
/// sink the script items come back through. Both are COM callbacks; their
/// vtables are built by hand in native memory with
/// <c>[UnmanagedCallersOnly]</c> methods, which trimming and AOT keep, rather
/// than through <c>ComWrappers</c>. DirectWrite only calls them during
/// <c>AnalyzeScript</c> and <c>MapCharacters</c>, so the object's lifetime is
/// this class's and reference counts are not kept.
/// </summary>
[SupportedOSPlatform("windows8.1")]
internal sealed unsafe class TextAnalysis : IDisposable
{
    // Same layout as a COM object: the source's vtable pointer first, then
    // the sink's, so each callback finds this block from its `this`.
    [StructLayout(LayoutKind.Sequential)]
    private struct Block
    {
        public IDWriteTextAnalysisSource.Vtbl* Source;
        public IDWriteTextAnalysisSink.Vtbl* Sink;
        public char* Text;
        public uint Length;
        public nint Owner;
        // A zero char: the empty locale name, the user's default.
        public char NoLocale;
        public IDWriteTextAnalysisSource.Vtbl SourceTable;
        public IDWriteTextAnalysisSink.Vtbl SinkTable;
    }

    private static readonly HRESULT Ok = (HRESULT)0, NoInterface = (HRESULT)unchecked((int)0x80004002);
    private static readonly Guid Unknown = new(0x00000000, 0x0000, 0x0000, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46);

    private Block* block;
    private GCHandle self;
    private readonly List<(uint Start, uint Length, DWRITE_SCRIPT_ANALYSIS Script)> scripts = [];

    public TextAnalysis()
    {
        block = (Block*)NativeMemory.AllocZeroed((nuint)sizeof(Block));
        self = GCHandle.Alloc(this, GCHandleType.Weak);
        block->Source = &block->SourceTable;
        block->Sink = &block->SinkTable;
        block->Owner = GCHandle.ToIntPtr(self);
        block->SourceTable.QueryInterface_1 = (delegate* unmanaged[Stdcall]<IDWriteTextAnalysisSource*, Guid*, void**, HRESULT>)&QueryInterface;
        block->SourceTable.AddRef_2 = (delegate* unmanaged[Stdcall]<IDWriteTextAnalysisSource*, uint>)&Count;
        block->SourceTable.Release_3 = (delegate* unmanaged[Stdcall]<IDWriteTextAnalysisSource*, uint>)&Count;
        block->SourceTable.GetTextAtPosition_4 = &TextAt;
        block->SourceTable.GetTextBeforePosition_5 = &TextBefore;
        block->SourceTable.GetParagraphReadingDirection_6 = &Direction;
        block->SourceTable.GetLocaleName_7 = &Locale;
        block->SourceTable.GetNumberSubstitution_8 = &Substitution;
        block->SinkTable.QueryInterface_1 = (delegate* unmanaged[Stdcall]<IDWriteTextAnalysisSink*, Guid*, void**, HRESULT>)&SinkQueryInterface;
        block->SinkTable.AddRef_2 = (delegate* unmanaged[Stdcall]<IDWriteTextAnalysisSink*, uint>)&Count;
        block->SinkTable.Release_3 = (delegate* unmanaged[Stdcall]<IDWriteTextAnalysisSink*, uint>)&Count;
        block->SinkTable.SetScriptAnalysis_4 = &SetScript;
        block->SinkTable.SetLineBreakpoints_5 = &Ignore;
        block->SinkTable.SetBidiLevel_6 = &IgnoreBidi;
        block->SinkTable.SetNumberSubstitution_7 = &IgnoreSubstitution;
    }

    public IDWriteTextAnalysisSource* Source => (IDWriteTextAnalysisSource*)block;

    public IDWriteTextAnalysisSink* Sink => (IDWriteTextAnalysisSink*)&block->Sink;

    /// <summary>Script items of the last <see cref="Analyse"/>, in text order.</summary>
    public IReadOnlyList<(uint Start, uint Length, DWRITE_SCRIPT_ANALYSIS Script)> Scripts => scripts;

    /// <summary>Points the source at <paramref name="text"/>, which must stay pinned until the next call.</summary>
    public void Use(char* text, int length)
    {
        block->Text = text;
        block->Length = (uint)length;
    }

    /// <summary>Splits the text <see cref="Use"/> set into script items.</summary>
    public void Analyse(IDWriteTextAnalyzer* analyzer)
    {
        scripts.Clear();
        analyzer->AnalyzeScript(Source, 0, block->Length, Sink);
        scripts.Sort(static (a, b) => a.Start.CompareTo(b.Start));
    }

    private static Block* Of(void* source) => (Block*)source;

    private static Block* OfSink(void* sink) => (Block*)((byte*)sink - sizeof(nint));

    private static HRESULT Answer(Block* b, Guid* iid, void** result)
    {
        if (*iid == Unknown || *iid == IDWriteTextAnalysisSource.IID_Guid)
        {
            *result = b;
            return Ok;
        }
        if (*iid == IDWriteTextAnalysisSink.IID_Guid)
        {
            *result = &b->Sink;
            return Ok;
        }
        *result = null;
        return NoInterface;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT QueryInterface(void* @this, Guid* iid, void** result) => Answer(Of(@this), iid, result);

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT SinkQueryInterface(void* @this, Guid* iid, void** result) => Answer(OfSink(@this), iid, result);

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static uint Count(void* @this) => 1;

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT TextAt(IDWriteTextAnalysisSource* @this, uint position, ushort** text, uint* length)
    {
        Block* b = Of(@this);
        bool inside = position < b->Length;
        *text = inside ? (ushort*)b->Text + position : null;
        *length = inside ? b->Length - position : 0;
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT TextBefore(IDWriteTextAnalysisSource* @this, uint position, ushort** text, uint* length)
    {
        Block* b = Of(@this);
        bool inside = position > 0 && position <= b->Length;
        *text = inside ? (ushort*)b->Text : null;
        *length = inside ? position : 0;
        return Ok;
    }

    // Logical order, as the grid holds it; the core does no bidi reordering.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static DWRITE_READING_DIRECTION Direction(IDWriteTextAnalysisSource* @this) => DWRITE_READING_DIRECTION.DWRITE_READING_DIRECTION_LEFT_TO_RIGHT;

    // The empty locale: the user's defaults pick the fallback, as for any untagged text.
    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT Locale(IDWriteTextAnalysisSource* @this, uint position, uint* length, ushort** locale)
    {
        Block* b = Of(@this);
        *length = position < b->Length ? b->Length - position : 0;
        *locale = (ushort*)&b->NoLocale;
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT Substitution(IDWriteTextAnalysisSource* @this, uint position, uint* length, IDWriteNumberSubstitution** substitution)
    {
        Block* b = Of(@this);
        *length = position < b->Length ? b->Length - position : 0;
        *substitution = null;
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT SetScript(IDWriteTextAnalysisSink* @this, uint start, uint length, DWRITE_SCRIPT_ANALYSIS* script)
    {
        if (GCHandle.FromIntPtr(OfSink(@this)->Owner).Target is TextAnalysis owner)
        {
            owner.scripts.Add((start, length, *script));
        }
        return Ok;
    }

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT Ignore(IDWriteTextAnalysisSink* @this, uint start, uint length, DWRITE_LINE_BREAKPOINT* breakpoints) => Ok;

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT IgnoreBidi(IDWriteTextAnalysisSink* @this, uint start, uint length, byte explicitLevel, byte resolvedLevel) => Ok;

    [UnmanagedCallersOnly(CallConvs = [typeof(CallConvStdcall)])]
    private static HRESULT IgnoreSubstitution(IDWriteTextAnalysisSink* @this, uint start, uint length, IDWriteNumberSubstitution* substitution) => Ok;

    public void Dispose()
    {
        if (block != null)
        {
            NativeMemory.Free(block);
            block = null;
            self.Free();
        }
    }
}
