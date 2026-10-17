using Microsoft.UI;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Windows.UI.Core;
using Scull.Core;

namespace Scull.App;

/// <summary>
/// The find bar over the terminal's top-right corner, as FindBar.swift lays it
/// out. It reports what the user did; the window owns the <see cref="FindState"/>.
/// </summary>
internal sealed partial class FindBar : UserControl
{
    private readonly TextBox field = new() { PlaceholderText = "Find", Width = 180 };
    private readonly ToggleButton caseButton = new() { Content = "Aa" };
    private readonly TextBlock status = new() { VerticalAlignment = VerticalAlignment.Center, Opacity = 0.7 };
    private bool quiet;

    public FindBar()
    {
        HorizontalAlignment = HorizontalAlignment.Right;
        VerticalAlignment = VerticalAlignment.Top;
        Visibility = Visibility.Collapsed;
        Margin = new Thickness(0, 8, 16, 0);
        ToolTipService.SetToolTip(caseButton, "Match Case");
        caseButton.Click += (_, _) => CaseToggled?.Invoke();
        field.TextChanged += (_, _) =>
        {
            if (!quiet)
            {
                Edited?.Invoke(field.Text);
            }
        };
        field.KeyDown += (_, e) =>
        {
            if (e.Key == VirtualKey.Enter)
            {
                bool shift = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift).HasFlag(CoreVirtualKeyStates.Down);
                Stepped?.Invoke(!shift);
                e.Handled = true;
            }
            else if (e.Key == VirtualKey.Escape)
            {
                // Escape ends the search outright, as on macOS.
                CloseRequested?.Invoke();
                e.Handled = true;
            }
        };
        Content = new Border
        {
            CornerRadius = new CornerRadius(8),
            Padding = new Thickness(8, 6, 8, 6),
            Background = new SolidColorBrush(ColorHelper.FromArgb(0xF0, 0x2B, 0x2B, 0x2B)),
            Child = new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Spacing = 6,
                Children =
                {
                    field, caseButton, status,
                    Icon("", "Previous", () => Stepped?.Invoke(false)),
                    Icon("", "Next", () => Stepped?.Invoke(true)),
                    Icon("", "Close", () => CloseRequested?.Invoke()),
                },
            },
        };
    }

    public event Action<string>? Edited;

    /// <summary>True for the next match, false for the previous one.</summary>
    public event Action<bool>? Stepped;

    public event Action? CaseToggled;

    public event Action? CloseRequested;

    private static Button Icon(string glyph, string name, Action click)
    {
        var button = new Button { Content = new FontIcon { Glyph = glyph, FontSize = 12 } };
        AutomationProperties.SetName(button, name);
        ToolTipService.SetToolTip(button, name);
        button.Click += (_, _) => click();
        return button;
    }

    /// <summary>Shows the bar on <paramref name="state"/>'s pattern with the keyboard in its field.</summary>
    public void Open(FindState state)
    {
        quiet = true;
        field.Text = state.Pattern;
        quiet = false;
        field.SelectAll();
        Visibility = Visibility.Visible;
        Show(state);
        field.Focus(FocusState.Programmatic);
    }

    public void Show(FindState state)
    {
        status.Text = state.Label;
        caseButton.IsChecked = !state.IgnoreCase;
    }

    public void Hide() => Visibility = Visibility.Collapsed;
}
