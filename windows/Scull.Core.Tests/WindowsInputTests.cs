namespace Scull.Core.Tests;

/// <summary>The Windows mapping as plain numbers, so it runs on every runner.</summary>
[TestClass]
public sealed class WindowsInputTests
{
    private const KeyModifiers Ctrl = KeyModifiers.Control;
    private const KeyModifiers AltGr = KeyModifiers.Control | KeyModifiers.Alt;

    private readonly List<KeyInput> keys = [];
    private readonly List<string> texts = [];
    private readonly WindowsKeyPairing pairing;

    public WindowsInputTests() => pairing = new WindowsKeyPairing(keys.Add, texts.Add);

    [TestMethod]
    [DataRow(0x1B, 0x01u, false, KeyCode.Escape)]
    [DataRow(0x0D, 0x1Cu, false, KeyCode.Enter)]
    [DataRow(0x0D, 0x1Cu, true, KeyCode.KeypadEnter)]
    [DataRow(0x25, 0x4Bu, true, KeyCode.Left)]
    [DataRow(0x25, 0x4Bu, false, KeyCode.KeypadLeft)]
    [DataRow(0x2E, 0x53u, true, KeyCode.Delete)]
    [DataRow(0x23, 0x4Fu, true, KeyCode.End)]
    [DataRow(0x23, 0x4Fu, false, KeyCode.Keypad0 + 25)]
    [DataRow(0x0C, 0x4Cu, false, KeyCode.Keypad0 + 28)]
    [DataRow(0x70, 0x3Bu, false, KeyCode.F1)]
    [DataRow(0x87, 0x00u, false, KeyCode.F1 + 23)]
    [DataRow(0x67, 0x48u, false, KeyCode.Keypad0 + 7)]
    [DataRow(0x6F, 0x35u, true, KeyCode.Keypad0 + 11)]
    [DataRow(0x6C, 0x00u, false, KeyCode.KeypadSeparator)]
    [DataRow(0x5D, 0x5Du, true, KeyCode.Menu)]
    [DataRow(0xAD, 0x20u, true, KeyCode.MediaPlay + 12)]
    [DataRow(0x10, 0x2Au, false, KeyCode.LeftShift)]
    [DataRow(0x10, 0x36u, false, KeyCode.RightShift)]
    [DataRow(0x11, 0x1Du, true, KeyCode.RightShift + 1)]
    [DataRow(0x12, 0x38u, false, KeyCode.LeftShift + 2)]
    [DataRow(0xA5, 0x38u, true, KeyCode.RightShift + 2)]
    [DataRow(0x5B, 0x5Bu, true, KeyCode.LeftShift + 3)]
    public void NamedKeysComeFromTheVirtualKey(int virtualKey, uint scanCode, bool extended, uint expected) =>
        Assert.AreEqual(expected, WindowsKeys.Map(virtualKey, scanCode, extended));

    [TestMethod]
    // A Russian layout's "с" key and a German layout's "z" key are still named by their US position.
    [DataRow(0x43, 0x2Eu, 'c')]
    [DataRow(0x5A, 0x15u, 'y')]
    [DataRow(0x31, 0x02u, '1')]
    [DataRow(0xBB, 0x0Du, '=')]
    [DataRow(0xDC, 0x2Bu, '\\')]
    [DataRow(0xC0, 0x29u, '`')]
    [DataRow(0xBF, 0x35u, '/')]
    [DataRow(0x20, 0x39u, ' ')]
    public void TheMainBlockComesFromTheScanCode(int virtualKey, uint scanCode, char expected) =>
        Assert.AreEqual(expected, WindowsKeys.Map(virtualKey, scanCode, extended: false));

    [TestMethod]
    [DataRow(0xE5, 0x1Eu, false)] // VK_PROCESSKEY: an input method has the key
    [DataRow(0xE2, 0x56u, false)] // the ISO key by Left Shift, absent from the US layout
    [DataRow(0xFF, 0x35u, true)] // an extended code outside the main block
    public void KeysWithNoNameMapToNull(int virtualKey, uint scanCode, bool extended) =>
        Assert.IsNull(WindowsKeys.Map(virtualKey, scanCode, extended));

    [TestMethod]
    public void PasteShortcutsIgnoreTheLocks()
    {
        Assert.IsTrue(WindowsKeys.IsPaste('v', Ctrl | KeyModifiers.Shift | KeyModifiers.CapsLock));
        Assert.IsTrue(WindowsKeys.IsPaste(KeyCode.Insert, KeyModifiers.Shift | KeyModifiers.NumLock));
        Assert.IsFalse(WindowsKeys.IsPaste('v', Ctrl));
        Assert.IsFalse(WindowsKeys.IsPaste(KeyCode.Insert, KeyModifiers.Shift | KeyModifiers.Alt));
        Assert.IsFalse(WindowsKeys.IsPaste(null, KeyModifiers.Shift));
    }

    [TestMethod]
    public void ATextKeyWaitsForItsCharacter()
    {
        Assert.IsFalse(pairing.KeyDown(0x41, 0x1E, false, false, KeyModifiers.Shift));
        Assert.IsEmpty(keys);
        pairing.Character('A');
        Assert.IsTrue(pairing.KeyUp(0x41, 0x1E, false, KeyModifiers.Shift));

        CollectionAssert.AreEqual(
            new[] { new KeyInput('a', KeyAction.Press, KeyModifiers.Shift, Text: "A"), new KeyInput('a', KeyAction.Release, KeyModifiers.Shift) },
            keys);
        Assert.IsEmpty(texts);
    }

    [TestMethod]
    public void ARepeatCarriesItsCharacterToo()
    {
        pairing.KeyDown(0x20, 0x39, false, true, KeyModifiers.None);
        pairing.Character(' ');

        Assert.AreEqual(new KeyInput(' ', KeyAction.Repeat, KeyModifiers.None, Text: " "), keys.Single());
    }

    [TestMethod]
    public void ControlGoesOutAtOnceAndItsCharacterIsDropped()
    {
        Assert.IsTrue(pairing.KeyDown(0x11, 0x1D, false, false, Ctrl));
        Assert.IsTrue(pairing.KeyDown(0x43, 0x2E, false, false, Ctrl));
        pairing.Character('\u0003');

        CollectionAssert.AreEqual(
            new[] { new KeyInput(KeyCode.LeftShift + 1, KeyAction.Press, Ctrl), new KeyInput('c', KeyAction.Press, Ctrl) },
            keys);
        Assert.IsEmpty(texts);
    }

    [TestMethod]
    public void NonTextKeysDropTheCharacterWindowsMakesOfThem()
    {
        Assert.IsTrue(pairing.KeyDown(0x0D, 0x1C, false, false, KeyModifiers.None));
        pairing.Character('\r');
        Assert.IsTrue(pairing.KeyDown(0x09, 0x0F, false, false, KeyModifiers.Shift));

        CollectionAssert.AreEqual(
            new[] { new KeyInput(KeyCode.Enter, KeyAction.Press, KeyModifiers.None), new KeyInput(KeyCode.Tab, KeyAction.Press, KeyModifiers.Shift) },
            keys);
    }

    [TestMethod]
    public void AltGrTypesItsCharacterWithCtrlAltConsumed()
    {
        // German AltGr+Q: Windows holds Left Control and Right Alt.
        pairing.KeyDown(0x51, 0x10, false, false, AltGr);
        pairing.Character('@');

        Assert.AreEqual(new KeyInput('q', KeyAction.Press, AltGr, AltGr, "@"), keys.Single());
    }

    [TestMethod]
    public void CtrlAltWithNoCharacterIsAChordSentWhenTheNextKeyEventComes()
    {
        pairing.KeyDown(0x41, 0x1E, false, false, AltGr);
        Assert.IsEmpty(keys);
        pairing.KeyUp(0x41, 0x1E, false, AltGr);

        CollectionAssert.AreEqual(
            new[] { new KeyInput('a', KeyAction.Press, AltGr), new KeyInput('a', KeyAction.Release, AltGr) },
            keys);
    }

    [TestMethod]
    public void ADeadKeyIsDroppedAndItsAccentComesWithTheNextKey()
    {
        // French AZERTY: the ^ dead key, then e, types ê.
        pairing.KeyDown(0xDD, 0x1A, false, false, KeyModifiers.None);
        pairing.KeyUp(0xDD, 0x1A, false, KeyModifiers.None);
        pairing.KeyDown(0x45, 0x12, false, false, KeyModifiers.None);
        pairing.Character('ê');

        CollectionAssert.AreEqual(
            new[] { new KeyInput('[', KeyAction.Release, KeyModifiers.None), new KeyInput('e', KeyAction.Press, KeyModifiers.None, Text: "ê") },
            keys);
    }

    [TestMethod]
    public void ASurrogatePairIsOneCharacter()
    {
        pairing.KeyDown(0x41, 0x1E, false, false, KeyModifiers.None);
        pairing.Character('\uD83D');
        pairing.Character('\uDE00');

        Assert.AreEqual("\U0001F600", keys.Single().Text);
    }

    [TestMethod]
    public void CharactersWithNoKeyGoOutAsTextAndLoneSurrogatesNot()
    {
        // An input method's commit: VK_PROCESSKEY, then characters.
        Assert.IsFalse(pairing.KeyDown(0xE5, 0x1E, false, false, KeyModifiers.None));
        pairing.Character('日');
        pairing.Character('\uDE00');
        pairing.Character('\uD83D');
        pairing.Character('x');

        Assert.AreEqual("日|x", string.Join('|', texts));
        Assert.IsEmpty(keys);
    }

    [TestMethod]
    public void AModifierCountsItselfAsHeld()
    {
        pairing.KeyDown(0x10, 0x2A, false, false, KeyModifiers.None);
        pairing.KeyUp(0x10, 0x2A, false, KeyModifiers.None);

        Assert.IsTrue(keys.All(k => k.Modifiers == KeyModifiers.Shift));
    }

    [TestMethod]
    public void WheelDeltasBecomeStepsAndCarryTheRest()
    {
        var wheel = new WheelSteps();

        Assert.AreEqual(2, wheel.Add(2 * WheelSteps.Notch));
        Assert.AreEqual(-1, wheel.Add(-WheelSteps.Notch));
        Assert.AreEqual(0, wheel.Add(80));
        Assert.AreEqual(1, wheel.Add(80));
        Assert.AreEqual(0, wheel.Add(-30));
        Assert.AreEqual(-1, wheel.Add(-130));
    }

    [TestMethod]
    [DataRow(0.0, 0.0, 0u, 0u)]
    [DataRow(25.0, 41.0, 2u, 2u)]
    [DataRow(-5.0, -1.0, 0u, 0u)]
    [DataRow(1e12, 1e12, 79u, 23u)]
    [DataRow(double.NaN, double.PositiveInfinity, 0u, 0u)]
    public void PixelsBecomeCellsClampedToTheGrid(double x, double y, uint col, uint row)
    {
        (uint c, uint r, uint px, uint py) = PointerCells.At(x, y, 10, 20, 80, 24);

        Assert.AreEqual((col, row), (c, r));
        Assert.AreEqual(double.IsFinite(x) ? (uint)Math.Clamp(x, 0, uint.MaxValue) : 0, px);
        Assert.AreEqual(double.IsFinite(y) ? (uint)Math.Clamp(y, 0, uint.MaxValue) : 0, py);
    }
}
