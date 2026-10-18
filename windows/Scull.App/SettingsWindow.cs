using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Scull.Core;

namespace Scull.App;

/// <summary>
/// The settings window, the twin of macos/Sources/Scull/SettingsView.swift: each control writes its
/// key into the config file through the core, which validates it and applies it everywhere at once.
/// It does not own the <see cref="Config"/> or poll it: whoever does calls <see cref="Refresh"/> after
/// each poll, which also moves the controls when the file is edited by hand.
/// </summary>
internal sealed class SettingsWindow : Window
{
    // A few families that ship with Windows; the box is editable for any other.
    private static readonly string[] Families = ["Cascadia Mono", "Cascadia Code", "Consolas", "Courier New", "Lucida Console"];

    private readonly Config config;
    private readonly ComboBox family = new() { IsEditable = true, Header = "Font family (empty for the system monospace)", ItemsSource = Families };
    private readonly NumberBox size = new()
    {
        Header = "Size (points)",
        Minimum = SettingsForm.MinFontSize,
        Maximum = SettingsForm.MaxFontSize,
        SmallChange = SettingsForm.FontSizeStep,
        SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline,
    };
    private readonly ComboBox scheme = new() { Header = "Colour scheme" };
    private readonly NumberBox scrollback = new()
    {
        Header = "Scrollback rows",
        Minimum = 0,
        Maximum = SettingsForm.MaxScrollback,
        SmallChange = 1000,
        SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline,
    };
    private readonly TextBlock message = new() { TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
    private readonly Button show = new() { Content = "Show config file" };
    private string path = "";
    private string fileError = "";
    private bool refreshing;

    public SettingsWindow(Config config)
    {
        ArgumentNullException.ThrowIfNull(config);
        this.config = config;
        Title = "Settings";
        AppWindow.Resize(new Windows.Graphics.SizeInt32(480, 560));
        message.Foreground = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Red);
        Content = new StackPanel
        {
            Spacing = 12,
            Padding = new Thickness(20),
            Children =
            {
                family,
                size,
                scheme,
                scrollback,
                new TextBlock { Text = "Scrollback applies to terminals opened afterwards.", Opacity = 0.7 },
                message,
                show,
            },
        };
        family.SelectionChanged += (_, _) =>
        {
            if (!refreshing && family.SelectedItem is string chosen)
            {
                Apply(SettingsForm.Family(chosen));
            }
        };
        // Enter, or leaving the box, submits what was typed.
        family.TextSubmitted += (_, e) =>
        {
            e.Handled = true;
            Apply(SettingsForm.Family(e.Text));
        };
        size.ValueChanged += (_, e) => Typed(SettingsForm.TryFontSize(e.NewValue, out SettingChange c, out string problem), c, problem);
        scrollback.ValueChanged += (_, e) => Typed(SettingsForm.TryScrollback(e.NewValue, out SettingChange c, out string problem), c, problem);
        scheme.SelectionChanged += (_, _) =>
        {
            if (!refreshing && scheme.SelectedItem is string chosen)
            {
                Apply(SettingsForm.Scheme(chosen));
            }
        };
        show.Click += (_, _) => ShowFile();
        if (config.Settings is { } current)
        {
            Refresh(current);
        }
    }

    /// <summary>Shows <paramref name="settings"/>: call it with each poll's result.</summary>
    public void Refresh(ConfigSettings settings)
    {
        refreshing = true;
        try
        {
            family.Text = settings.FontFamily;
            size.Value = settings.FontSize;
            scrollback.Value = settings.Scrollback;
            if (!SettingsForm.Schemes(settings.Scheme).SequenceEqual(scheme.Items.OfType<string>()))
            {
                scheme.ItemsSource = SettingsForm.Schemes(settings.Scheme);
            }
            scheme.SelectedItem = settings.Scheme;
            path = settings.Path;
            fileError = settings.Error;
            show.IsEnabled = path.Length > 0;
            message.Text = fileError;
        }
        finally
        {
            refreshing = false;
        }
    }

    private void Typed(bool valid, SettingChange change, string problem)
    {
        if (refreshing)
        {
            return;
        }
        if (valid)
        {
            Apply(change);
            return;
        }
        message.Text = problem;
        SnapBack();
    }

    private void Apply(SettingChange change)
    {
        string? refusal = SettingsForm.Write(config, change);
        message.Text = refusal ?? fileError;
        if (refusal is not null)
        {
            SnapBack();
        }
    }

    // A value the core refused leaves the setting, and the control returns to it.
    private void SnapBack()
    {
        if (config.Settings is { } current)
        {
            string shown = message.Text;
            Refresh(current);
            message.Text = shown;
        }
    }

    private void ShowFile()
    {
        if (path.Length > 0)
        {
            // An argument list, not a command line: the path is not parsed by a shell.
            Process.Start(new ProcessStartInfo("explorer.exe") { ArgumentList = { "/select," + path } });
        }
    }
}
