using System.Runtime.InteropServices;
using System.Runtime.Versioning;
using Windows.Win32.Foundation;
using Windows.Win32.Graphics.DirectWrite;
using Windows.Win32.System.Com;

namespace Scull.Render;

/// <summary>
/// The Symbols Nerd Font Mono shipped beside the assembly, as SymbolFont.swift
/// bundles it (T27): prompt themes print Nerd Font icons from the Private Use
/// Area, and no Windows font has them. Segoe Fluent Icons has other icons
/// there, so this font is mapped before the system fallback, never after.
/// </summary>
// IDWriteFactory3's font sets are what load a font file without registering it.
[SupportedOSPlatform("windows10.0.10240")]
internal sealed unsafe class SymbolFont : IDisposable
{
    public const string Family = "Symbols Nerd Font Mono";

    // The BMP's Private Use Area, and plane 15's, where the Material Design icons are.
    private static readonly DWRITE_UNICODE_RANGE[] Ranges =
        [new() { first = 0xE000, last = 0xF8FF }, new() { first = 0xF0000, last = 0xFFFFD }];

    private IDWriteFontCollection1* collection;

    private SymbolFont(IDWriteFontCollection1* collection) => this.collection = collection;

    /// <summary>Where the build puts the font: <c>Fonts</c> next to the assembly.</summary>
    public static string BundledPath => Path.Combine(AppContext.BaseDirectory, "Fonts", "SymbolsNerdFontMono-Regular.ttf");

    private IDWriteFontCollection* Collection => (IDWriteFontCollection*)collection;

    /// <summary>The font in <paramref name="path"/> as a private collection; null when the file is missing or no such font, which only loses the icons.</summary>
    public static SymbolFont? Load(IDWriteFactory2* factory, string path)
    {
        if (!File.Exists(path))
        {
            return null;
        }
        IDWriteFactory3* factory3 = null;
        IDWriteFontFaceReference* reference = null;
        IDWriteFontSetBuilder* builder = null;
        IDWriteFontSet* set = null;
        try
        {
            Guid id = IDWriteFactory3.IID_Guid;
            ((IUnknown*)factory)->QueryInterface(&id, (void**)&factory3).ThrowOnFailure();
            fixed (char* p = path)
            {
                factory3->CreateFontFaceReference(p, null, 0, DWRITE_FONT_SIMULATIONS.DWRITE_FONT_SIMULATIONS_NONE, &reference);
            }
            factory3->CreateFontSetBuilder(&builder);
            builder->AddFontFaceReference(reference);
            builder->CreateFontSet(&set);
            IDWriteFontCollection1* made;
            factory3->CreateFontCollectionFromFontSet(set, &made);
            var font = new SymbolFont(made);
            if (!font.HasFamily())
            {
                // A file that is not this font reads as an empty collection.
                font.Dispose();
                return null;
            }
            return font;
        }
        catch (COMException)
        {
            return null;
        }
        finally
        {
            Com.Release(ref set);
            Com.Release(ref builder);
            Com.Release(ref reference);
            Com.Release(ref factory3);
        }
    }

    private bool HasFamily()
    {
        uint index;
        BOOL exists;
        fixed (char* name = Family)
        {
            Collection->FindFamilyName(name, &index, &exists);
        }
        return exists;
    }

    /// <summary>
    /// The system's fallback with this font mapped first for the Private Use
    /// Area. The face is still tried before either, so the order is T27's.
    /// The caller releases it; null when DirectWrite refuses the mapping, and the system's stays.
    /// </summary>
    public IDWriteFontFallback* Fallback(IDWriteFactory2* factory)
    {
        IDWriteFontFallbackBuilder* builder = null;
        IDWriteFontFallback* system = null;
        try
        {
            factory->CreateFontFallbackBuilder(&builder);
            fixed (char* family = Family)
            fixed (DWRITE_UNICODE_RANGE* ranges = Ranges)
            {
                var name = (ushort*)family;
                builder->AddMapping(ranges, (uint)Ranges.Length, &name, 1, Collection, default, default, 1);
            }
            factory->GetSystemFontFallback(&system);
            builder->AddMappings(system);
            IDWriteFontFallback* made;
            builder->CreateFontFallback(&made);
            return made;
        }
        catch (COMException)
        {
            return null;
        }
        finally
        {
            Com.Release(ref system);
            Com.Release(ref builder);
        }
    }

    /// <summary>Whether <paramref name="face"/> is this font's.</summary>
    public bool Owns(IDWriteFontFace* face)
    {
        IDWriteFont* font = null;
        try
        {
            Collection->GetFontFromFontFace(face, &font);
            return true;
        }
        catch (COMException)
        {
            // DWRITE_E_NOFONT: a face from another collection.
            return false;
        }
        finally
        {
            Com.Release(ref font);
        }
    }

    public void Dispose() => Com.Release(ref collection);
}
