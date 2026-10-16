using System.Diagnostics.CodeAnalysis;

namespace Scull.Render;

/// <summary>
/// Shaped runs by key, bounded as RunShaper.swift bounds them: two
/// generations, where a hit in the old one moves forward and the old one is
/// dropped whole when the new one fills, so what is still on screen stays.
/// Looked up by span, so a hit allocates nothing.
/// </summary>
internal sealed class RunCache<T>
    where T : class
{
    private Dictionary<string, T> recent = [], older = [];
    private readonly int limit;

    public RunCache(int limit = 4096) => this.limit = Math.Max(1, limit);

    public int Count => recent.Count + older.Count;

    public bool TryGet(ReadOnlySpan<char> key, [MaybeNullWhen(false)] out T value)
    {
        if (recent.GetAlternateLookup<ReadOnlySpan<char>>().TryGetValue(key, out value))
        {
            return true;
        }
        if (!older.GetAlternateLookup<ReadOnlySpan<char>>().Remove(key, out string? actual, out value))
        {
            return false;
        }
        Keep(actual, value);
        return true;
    }

    public void Add(ReadOnlySpan<char> key, T value) => Keep(key.ToString(), value);

    private void Keep(string key, T value)
    {
        if (recent.Count >= limit)
        {
            // Cleared rather than replaced, so the tables keep their capacity.
            (older, recent) = (recent, older);
            recent.Clear();
        }
        recent[key] = value;
    }
}
