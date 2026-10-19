namespace Scull.Render;

/// <summary>A rectangle of a render target, in pixels from its top-left corner.</summary>
public readonly record struct Region(int X, int Y, int Width, int Height);
