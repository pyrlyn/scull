namespace Scull.Bench;

/// <summary>The T3 payload.</summary>
internal static class BenchInput
{
    public const int Bytes = 1_000_000;
    public const int Columns = 80;

    /// <summary>
    /// <c>scull_bench::fixed_input</c>: A to Z, a line feed closing every 80 bytes. A copy because the
    /// generator is Rust; both tests pin <see cref="Hash"/> of it, so the copies cannot drift apart.
    /// </summary>
    public static byte[] Fixed()
    {
        var output = new byte[Bytes];
        int letter = 0;
        for (int i = 0; i < output.Length; i++)
        {
            if ((i + 1) % Columns == 0)
            {
                output[i] = (byte)'\n';
            }
            else
            {
                output[i] = (byte)('A' + (letter++ % 26));
            }
        }
        return output;
    }

    /// <summary>FNV-1a, 64 bits.</summary>
    public static ulong Hash(ReadOnlySpan<byte> bytes)
    {
        ulong hash = 0xcbf29ce484222325;
        foreach (byte b in bytes)
        {
            hash = (hash ^ b) * 0x100000001b3;
        }
        return hash;
    }
}
