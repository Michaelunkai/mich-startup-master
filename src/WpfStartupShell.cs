using Microsoft.Win32;
using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Data;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using FormsContextMenuStrip = System.Windows.Forms.ContextMenuStrip;
using FormsNotifyIcon = System.Windows.Forms.NotifyIcon;

namespace MichStartupMaster
{
    internal static class WpfStartupShell
    {
        public static int Run(bool startInTray)
        {
            var app = new System.Windows.Application { ShutdownMode = ShutdownMode.OnMainWindowClose };
            app.Resources[SystemFonts.MessageFontFamilyKey] = new FontFamily("Segoe UI");
            var window = new StartupWindow(startInTray);
            app.MainWindow = window;
            return app.Run(window);
        }

        public static void ShowAddDialog()
        {
            var dialog = new StartupEditWindow();
            dialog.ShowDialog();
        }

        public static string UiContractJson()
        {
            return "{\"frontend\":\"WPF\",\"wpf\":true,\"fastFirstPaint\":true,\"cachedFirstPaint\":true,\"virtualizedGrid\":true,\"columns\":[\"Status\",\"Application\",\"Startup entry\",\"Source\",\"Risk\",\"Cleanup\",\"Popup\",\"Location\",\"Launch command\"],\"popupEnabledLabel\":\"Enabled\",\"popupDisabledLabel\":\"Disabled\",\"popupNotApplicableLabel\":\"N/A\",\"oneClickPopupToggle\":true,\"trayIcon\":true,\"trayDoubleClickOpens\":true,\"startInTrayArgument\":\"--start-in-tray\",\"asyncRefresh\":true,\"humanReadableNames\":true,\"greenCleanupAdvice\":true,\"contextMenu\":true,\"keyboardShortcuts\":true,\"filters\":[\"All\",\"High risk\",\"Suggested cleanup\",\"Disabled\"],\"tools\":[\"Add startup\",\"Edit startup\",\"Remove startup\",\"Restore startup\",\"Make quiet\",\"Launch now\",\"Open location\",\"Copy command\",\"Protect disabled\",\"Enforce now\",\"Open startup folders\"]}";
        }
    }

    internal sealed class StartupWindow : Window
    {
        private readonly ObservableCollection<StartupRow> _rows = new ObservableCollection<StartupRow>();
        private readonly ICollectionView _view;
        private readonly bool _startInTray;
        private readonly string _cachePath = Path.Combine(Program.AppData, "last-inventory.tsv");
        private DataGrid _grid;
        private TextBox _search;
        private TextBlock _summary;
        private TextBlock _hint;
        private TextBlock _visibleValue, _enabledValue, _disabledValue, _cleanupValue, _managedValue;
        private string _filter = "All";
        private FormsNotifyIcon _tray;
        private bool _reallyExit;
        private bool _refreshing;

        private static readonly Brush Page = BrushOf("#0a111c");
        private static readonly Brush Surface = BrushOf("#101a28");
        private static readonly Brush Surface2 = BrushOf("#172436");
        private static readonly Brush Border = BrushOf("#26364a");
        private static readonly Brush Text = BrushOf("#f7fbff");
        private static readonly Brush Muted = BrushOf("#9aa8b7");
        private static readonly Brush Accent = BrushOf("#16b8a6");
        private static readonly Brush Blue = BrushOf("#3b82f6");
        private static readonly Brush Good = BrushOf("#34d399");
        private static readonly Brush Warn = BrushOf("#f59e0b");
        private static readonly Brush Bad = BrushOf("#ef4444");

        public StartupWindow(bool startInTray)
        {
            _startInTray = startInTray;
            Title = "Mich Startup Master - Windows Boot Control";
            Width = 1480;
            Height = 920;
            MinWidth = 1120;
            MinHeight = 720;
            Background = Page;
            Foreground = Text;
            WindowStartupLocation = WindowStartupLocation.CenterScreen;
            Icon = IconSource();
            BuildUi();
            _view = CollectionViewSource.GetDefaultView(_rows);
            _view.Filter = FilterRow;
            _grid.ItemsSource = _view;
            Loaded += async (s, e) => await LoadInventoryAsync();
            ContentRendered += (s, e) => { if (_startInTray) HideToTray(); };
            Closing += OnClosing;
            KeyDown += OnKeyDown;
            BuildTray();
        }

        private void BuildUi()
        {
            var root = new DockPanel { LastChildFill = true, Margin = new Thickness(18) };
            Content = root;

            var header = PanelCard(138);
            DockPanel.SetDock(header, Dock.Top);
            root.Children.Add(header);
            var headerGrid = new Grid();
            headerGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            headerGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            header.Child = headerGrid;
            var titleStack = new StackPanel { Margin = new Thickness(22, 16, 16, 16) };
            headerGrid.Children.Add(titleStack);
            titleStack.Children.Add(new TextBlock { Text = "Startup Master", FontSize = 34, FontWeight = FontWeights.SemiBold, Foreground = Text });
            titleStack.Children.Add(new TextBlock { Text = "Fast control for Windows startup apps, scheduled tasks, services, drivers, registry entries, startup folders, and quiet tray launch.", FontSize = 14, Foreground = Muted, Margin = new Thickness(0, 6, 0, 0) });
            _summary = new TextBlock { FontSize = 13, FontWeight = FontWeights.SemiBold, Foreground = Good, Margin = new Thickness(0, 14, 0, 0), Text = "Opening instantly. Inventory scan starts in the background." };
            titleStack.Children.Add(_summary);
            var actionStack = new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(8, 0, 22, 0), VerticalAlignment = VerticalAlignment.Center };
            Grid.SetColumn(actionStack, 1);
            headerGrid.Children.Add(actionStack);
            actionStack.Children.Add(ActionButton("Add", Accent, (s, e) => AddStartup()));
            actionStack.Children.Add(ActionButton("Refresh", Blue, async (s, e) => await RefreshInventoryAsync(true)));
            actionStack.Children.Add(ActionButton("Enforce", Surface2, async (s, e) => await RunGuardsAsync(true)));

            var metrics = new UniformGrid { Columns = 5, Margin = new Thickness(0, 14, 0, 14) };
            DockPanel.SetDock(metrics, Dock.Top);
            root.Children.Add(metrics);
            _visibleValue = Metric(metrics, "Visible", "current view", Blue);
            _enabledValue = Metric(metrics, "Enabled", "runs at logon", Good);
            _disabledValue = Metric(metrics, "Disabled", "blocked from boot", Bad);
            _cleanupValue = Metric(metrics, "Cleanup", "safe candidates", Good);
            _managedValue = Metric(metrics, "Managed", "created here", Accent);

            var toolbar = PanelCard(96);
            DockPanel.SetDock(toolbar, Dock.Top);
            root.Children.Add(toolbar);
            var toolGrid = new Grid { Margin = new Thickness(16, 12, 16, 12) };
            toolGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(430) });
            toolGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            toolGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            toolbar.Child = toolGrid;

            var searchStack = new StackPanel();
            toolGrid.Children.Add(searchStack);
            searchStack.Children.Add(new TextBlock { Text = "Search", Foreground = Muted, FontSize = 12, FontWeight = FontWeights.SemiBold });
            _search = new TextBox { Height = 34, Margin = new Thickness(0, 6, 16, 0), Background = BrushOf("#0b1422"), Foreground = Text, BorderBrush = Border, CaretBrush = Text, FontSize = 14, Padding = new Thickness(10, 5, 10, 5) };
            _search.TextChanged += (s, e) => RefreshView();
            searchStack.Children.Add(_search);

            var filters = new StackPanel { Orientation = Orientation.Horizontal, VerticalAlignment = VerticalAlignment.Bottom };
            Grid.SetColumn(filters, 1);
            toolGrid.Children.Add(filters);
            filters.Children.Add(FilterButton("All"));
            filters.Children.Add(FilterButton("High risk"));
            filters.Children.Add(FilterButton("Suggested cleanup"));
            filters.Children.Add(FilterButton("Disabled"));

            var tools = new WrapPanel { HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center, MaxWidth = 610 };
            Grid.SetColumn(tools, 2);
            toolGrid.Children.Add(tools);
            tools.Children.Add(ActionButton("Edit", Surface2, (s, e) => EditSelected()));
            tools.Children.Add(ActionButton("Remove", Bad, (s, e) => DisableSelected()));
            tools.Children.Add(ActionButton("Restore", Good, (s, e) => EnableSelected()));
            tools.Children.Add(ActionButton("Quiet", Accent, (s, e) => MakeSelectedQuiet()));
            tools.Children.Add(ActionButton("Folders", Surface2, (s, e) => OpenStartupFolders()));

            var gridWrap = PanelCard(double.NaN);
            root.Children.Add(gridWrap);
            var gridDock = new DockPanel { Margin = new Thickness(14) };
            gridWrap.Child = gridDock;
            _hint = new TextBlock { Text = "Loading cached inventory, then live startup state.", Foreground = Muted, FontSize = 13, Margin = new Thickness(2, 0, 0, 10) };
            DockPanel.SetDock(_hint, Dock.Top);
            gridDock.Children.Add(_hint);
            _grid = new DataGrid
            {
                ItemsSource = null,
                AutoGenerateColumns = false,
                IsReadOnly = true,
                SelectionMode = DataGridSelectionMode.Single,
                GridLinesVisibility = DataGridGridLinesVisibility.None,
                HeadersVisibility = DataGridHeadersVisibility.Column,
                EnableRowVirtualization = true,
                EnableColumnVirtualization = true,
                Background = BrushOf("#0b1422"),
                Foreground = Text,
                RowBackground = BrushOf("#0f1b2b"),
                AlternatingRowBackground = BrushOf("#122034"),
                BorderBrush = Border,
                HorizontalGridLinesBrush = Border,
                VerticalGridLinesBrush = Border,
                FontSize = 13,
                RowHeight = 42
            };
            VirtualizingPanel.SetIsVirtualizing(_grid, true);
            VirtualizingPanel.SetVirtualizationMode(_grid, VirtualizationMode.Recycling);
            _grid.MouseDoubleClick += (s, e) => EditSelected();
            _grid.AddHandler(ButtonBase.ClickEvent, new RoutedEventHandler(OnGridButtonClick));
            _grid.ContextMenu = BuildContextMenu();
            AddColumns();
            gridDock.Children.Add(_grid);
        }

        private void AddColumns()
        {
            _grid.Columns.Add(TextColumn("Status", "Status", 92));
            _grid.Columns.Add(TextColumn("Application", "Application", 230));
            _grid.Columns.Add(TextColumn("Startup entry", "Entry", 210));
            _grid.Columns.Add(TextColumn("Source", "Source", 135));
            _grid.Columns.Add(TextColumn("Risk", "Risk", 105));
            _grid.Columns.Add(TextColumn("Cleanup", "Cleanup", 112));
            var factory = new FrameworkElementFactory(typeof(System.Windows.Controls.Button));
            factory.SetBinding(ContentControl.ContentProperty, new Binding("Popup"));
            factory.SetValue(Control.PaddingProperty, new Thickness(10, 3, 10, 3));
            factory.SetValue(Control.ForegroundProperty, BrushOf("#07111c"));
            factory.SetValue(Control.FontWeightProperty, FontWeights.SemiBold);
            factory.SetValue(Control.BorderThicknessProperty, new Thickness(0));
            factory.SetBinding(Control.BackgroundProperty, new Binding("PopupBrush"));
            _grid.Columns.Add(new DataGridTemplateColumn { Header = "Popup", Width = 112, CellTemplate = new DataTemplate { VisualTree = factory } });
            _grid.Columns.Add(TextColumn("Location", "Location", 260));
            _grid.Columns.Add(TextColumn("Launch command", "Command", 520));
        }

        private DataGridTextColumn TextColumn(string header, string binding, double width)
        {
            return new DataGridTextColumn
            {
                Header = header,
                Binding = new Binding(binding),
                Width = new DataGridLength(width),
                ElementStyle = new Style(typeof(TextBlock))
                {
                    Setters =
                    {
                        new Setter(TextBlock.TextTrimmingProperty, TextTrimming.CharacterEllipsis),
                        new Setter(TextBlock.VerticalAlignmentProperty, VerticalAlignment.Center),
                        new Setter(TextBlock.MarginProperty, new Thickness(8,0,8,0))
                    }
                }
            };
        }

        private async Task LoadInventoryAsync()
        {
            LoadCache();
            await RefreshInventoryAsync(false);
        }

        private async Task RefreshInventoryAsync(bool userRequested)
        {
            if (_refreshing) return;
            _refreshing = true;
            _hint.Text = userRequested ? "Refreshing live startup inventory..." : "Live scan running in the background...";
            try
            {
                var items = await Task.Run(() =>
                {
                    ProtectedDisabledService.EnforceProtected();
                    ProtectedQuietService.EnforceProtected();
                    var scanned = StartupService.ScanAll();
                    ProtectedDisabledService.ProtectCurrentDisabled();
                    return scanned;
                });
                ReplaceRows(items);
                SaveCache(items);
                _hint.Text = "Live inventory loaded. Use Add, Edit, Remove, Restore, Quiet, or right-click any row.";
            }
            catch (Exception ex)
            {
                _hint.Text = "Inventory refresh failed: " + ex.Message;
                System.Windows.MessageBox.Show(ex.Message, "Refresh failed", MessageBoxButton.OK, MessageBoxImage.Warning);
            }
            finally { _refreshing = false; }
        }

        private void ReplaceRows(IEnumerable<StartupItem> items)
        {
            _rows.Clear();
            foreach (var item in items) _rows.Add(new StartupRow(item));
            RefreshView();
        }

        private void RefreshView()
        {
            if (_view != null) _view.Refresh();
            int visible = _view == null ? _rows.Count : _view.Cast<object>().Count();
            int enabled = _rows.Count(x => x.Enabled);
            int disabled = _rows.Count - enabled;
            int cleanup = _rows.Count(x => x.Cleanup == "Remove?");
            int managed = _rows.Count(x => x.IsManaged);
            _summary.Text = visible + " visible / " + _rows.Count + " total | " + enabled + " enabled | " + disabled + " disabled | " + cleanup + " cleanup suggestions";
            _visibleValue.Text = visible.ToString();
            _enabledValue.Text = enabled.ToString();
            _disabledValue.Text = disabled.ToString();
            _cleanupValue.Text = cleanup.ToString();
            _managedValue.Text = managed.ToString();
        }

        private bool FilterRow(object value)
        {
            var row = value as StartupRow;
            if (row == null) return false;
            if (_filter == "High risk" && row.Risk != "HIGH RISK") return false;
            if (_filter == "Suggested cleanup" && row.Cleanup != "Remove?") return false;
            if (_filter == "Disabled" && row.Enabled && row.Popup != "Disabled") return false;
            string q = (_search == null ? "" : _search.Text ?? "").Trim();
            if (q.Length == 0) return true;
            return row.SearchText.IndexOf(q, StringComparison.OrdinalIgnoreCase) >= 0;
        }

        private void AddStartup()
        {
            var dialog = new StartupEditWindow { Owner = this };
            if (dialog.ShowDialog() != true) return;
            try
            {
                StartupService.AddManagedStartup(dialog.AppTitle, dialog.AppPath, dialog.AppArguments, dialog.TrayMode, true);
                _hint.Text = "Added startup item: " + dialog.AppTitle;
                RefreshInventoryAsync(false);
            }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Add failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void EditSelected()
        {
            var row = Selected();
            if (row == null || row.Popup == "N/A") return;
            try
            {
                string target, args;
                StartupService.ResolveLaunchTarget(row.Item, out target, out args);
                var dialog = new StartupEditWindow(row.Application, target, args, row.Popup != "Enabled") { Owner = this };
                if (dialog.ShowDialog() != true) return;
                StartupService.EditStartup(row.Item, dialog.AppTitle, dialog.AppPath, dialog.AppArguments, dialog.TrayMode);
                _hint.Text = "Saved startup item: " + dialog.AppTitle;
                RefreshInventoryAsync(false);
            }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Edit failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void DisableSelected()
        {
            var row = Selected();
            if (row == null || !row.Enabled || !row.CanDisable) return;
            if (System.Windows.MessageBox.Show("Remove '" + row.Entry + "' from Windows startup?", "Confirm remove", MessageBoxButton.YesNo, MessageBoxImage.Question) != MessageBoxResult.Yes) return;
            try { StartupService.Disable(row.Item); _hint.Text = "Removed from startup: " + row.Entry; RefreshInventoryAsync(false); }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Remove failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void EnableSelected()
        {
            var row = Selected();
            if (row == null || row.Enabled) return;
            try { StartupService.Enable(row.Item); _hint.Text = "Restored startup item: " + row.Entry; RefreshInventoryAsync(false); }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Restore failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void MakeSelectedQuiet()
        {
            var row = Selected();
            if (row == null || row.Popup == "N/A") return;
            try { StartupService.SetPopupMode(row.Item, false); _hint.Text = "Quiet startup protected: " + row.Entry; RefreshInventoryAsync(false); }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Quiet mode failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void TogglePopup(StartupRow row)
        {
            if (row == null || row.Popup == "N/A") return;
            try { StartupService.TogglePopupMode(row.Item); _hint.Text = "Popup state toggled for: " + row.Entry; RefreshInventoryAsync(false); }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Popup toggle failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void DeleteManaged()
        {
            var row = Selected();
            if (row == null || !row.IsManaged || row.Source != "Scheduled Task") return;
            if (System.Windows.MessageBox.Show("Delete managed startup task '" + row.Entry + "'?", "Confirm delete", MessageBoxButton.YesNo, MessageBoxImage.Warning) != MessageBoxResult.Yes) return;
            try { StartupService.DeleteManagedTask(row.Location); _hint.Text = "Deleted managed task: " + row.Entry; RefreshInventoryAsync(false); }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Delete task failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void LaunchSelectedNow()
        {
            var row = Selected();
            if (row == null) return;
            try
            {
                string target, args, execute, actionArgs;
                StartupService.ResolveLaunchTarget(row.Item, out target, out args);
                StartupService.BuildDirectAction(target, args, out execute, out actionArgs);
                Process.Start(new ProcessStartInfo(execute, actionArgs) { UseShellExecute = false, WorkingDirectory = Directory.Exists(Path.GetDirectoryName(target)) ? Path.GetDirectoryName(target) : Environment.GetFolderPath(Environment.SpecialFolder.UserProfile) });
                _hint.Text = "Launched: " + row.Application;
            }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Launch failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void OpenSelectedLocation()
        {
            var row = Selected();
            if (row == null) return;
            try
            {
                string target, args;
                StartupService.ResolveLaunchTarget(row.Item, out target, out args);
                if (File.Exists(target)) Process.Start("explorer.exe", "/select,\"" + target + "\"");
                else if (Directory.Exists(row.Location)) Process.Start("explorer.exe", row.Location);
            }
            catch (Exception ex) { System.Windows.MessageBox.Show(ex.Message, "Open location failed", MessageBoxButton.OK, MessageBoxImage.Warning); }
        }

        private void CopySelectedCommand()
        {
            var row = Selected();
            if (row == null) return;
            System.Windows.Clipboard.SetText(row.Command ?? "");
            _hint.Text = "Copied launch command.";
        }

        private async Task RunGuardsAsync(bool showResult)
        {
            try
            {
                string result = await Task.Run(() => ProtectedDisabledService.EnforceProtected() + " | " + ProtectedQuietService.EnforceProtected());
                if (showResult) _hint.Text = result;
            }
            catch (Exception ex) { if (showResult) _hint.Text = "Guard failed: " + ex.Message; }
        }

        private void OpenStartupFolders()
        {
            Process.Start("explorer.exe", Environment.GetFolderPath(Environment.SpecialFolder.Startup));
            Process.Start("explorer.exe", Environment.GetFolderPath(Environment.SpecialFolder.CommonStartup));
        }

        private StartupRow Selected()
        {
            return _grid == null ? null : _grid.SelectedItem as StartupRow;
        }

        private void OnGridButtonClick(object sender, RoutedEventArgs e)
        {
            var button = e.OriginalSource as System.Windows.Controls.Button;
            if (button == null) return;
            TogglePopup(button.DataContext as StartupRow);
            e.Handled = true;
        }

        private void OnKeyDown(object sender, System.Windows.Input.KeyEventArgs e)
        {
            if (e.Key == Key.F5) { RefreshInventoryAsync(true); e.Handled = true; }
            else if (e.Key == Key.Delete) { DisableSelected(); e.Handled = true; }
            else if (e.Key == Key.Enter) { EditSelected(); e.Handled = true; }
            else if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.N) { AddStartup(); e.Handled = true; }
            else if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.L) { LaunchSelectedNow(); e.Handled = true; }
            else if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.O) { OpenSelectedLocation(); e.Handled = true; }
            else if (Keyboard.Modifiers == ModifierKeys.Control && e.Key == Key.C) { CopySelectedCommand(); e.Handled = true; }
            else if (e.Key == Key.Escape && _search != null && !string.IsNullOrWhiteSpace(_search.Text)) { _search.Text = ""; e.Handled = true; }
        }

        private ContextMenu BuildContextMenu()
        {
            var menu = new ContextMenu();
            menu.Items.Add(MenuItem("Edit startup", (s, e) => EditSelected()));
            menu.Items.Add(MenuItem("Remove startup", (s, e) => DisableSelected()));
            menu.Items.Add(MenuItem("Restore startup", (s, e) => EnableSelected()));
            menu.Items.Add(new Separator());
            menu.Items.Add(MenuItem("Make quiet", (s, e) => MakeSelectedQuiet()));
            menu.Items.Add(MenuItem("Delete managed task", (s, e) => DeleteManaged()));
            menu.Items.Add(MenuItem("Launch now", (s, e) => LaunchSelectedNow()));
            menu.Items.Add(MenuItem("Open location", (s, e) => OpenSelectedLocation()));
            menu.Items.Add(MenuItem("Copy command", (s, e) => CopySelectedCommand()));
            return menu;
        }

        private MenuItem MenuItem(string text, RoutedEventHandler handler)
        {
            var item = new MenuItem { Header = text };
            item.Click += handler;
            return item;
        }

        private Button ActionButton(string text, Brush bg, RoutedEventHandler click)
        {
            var b = new Button { Content = text, Height = 36, MinWidth = 74, Margin = new Thickness(5), Background = bg, Foreground = Text, BorderBrush = Border, Padding = new Thickness(12, 0, 12, 0), FontWeight = FontWeights.SemiBold };
            b.Click += click;
            return b;
        }

        private Button FilterButton(string text)
        {
            var b = ActionButton(text, text == _filter ? Accent : Surface2, (s, e) =>
            {
                _filter = text;
                foreach (var child in ((Panel)((Button)s).Parent).Children.OfType<Button>()) child.Background = Surface2;
                ((Button)s).Background = Accent;
                RefreshView();
            });
            b.MinWidth = text == "Suggested cleanup" ? 142 : 92;
            return b;
        }

        private Border PanelCard(double height)
        {
            return new Border { Height = double.IsNaN(height) ? double.NaN : height, Background = Surface, BorderBrush = Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Margin = new Thickness(0, 0, 0, 10) };
        }

        private TextBlock Metric(Panel host, string label, string helper, Brush accent)
        {
            var card = new Border { Background = Surface, BorderBrush = Border, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Margin = new Thickness(0, 0, 10, 0), Padding = new Thickness(16, 10, 16, 10) };
            var stack = new StackPanel();
            card.Child = stack;
            var value = new TextBlock { Text = "0", FontSize = 26, FontWeight = FontWeights.SemiBold, Foreground = Text };
            stack.Children.Add(value);
            stack.Children.Add(new TextBlock { Text = label, Foreground = accent, FontWeight = FontWeights.SemiBold, FontSize = 13 });
            stack.Children.Add(new TextBlock { Text = helper, Foreground = Muted, FontSize = 12 });
            host.Children.Add(card);
            return value;
        }

        private void LoadCache()
        {
            try
            {
                if (!File.Exists(_cachePath)) return;
                var cached = File.ReadAllLines(_cachePath).Select(CacheDecode).Where(x => x != null).ToList();
                if (cached.Count > 0)
                {
                    ReplaceRows(cached);
                    _hint.Text = "Showing cached inventory while live scan finishes.";
                }
            }
            catch { }
        }

        private void SaveCache(IEnumerable<StartupItem> items)
        {
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(_cachePath));
                File.WriteAllLines(_cachePath, items.Take(1200).Select(CacheEncode));
            }
            catch { }
        }

        private static string CacheEncode(StartupItem x)
        {
            string[] f = { x.Id, x.Name, x.Source, x.Scope, x.Command, x.Location, x.AppName, x.Enabled.ToString(), x.CanDisable.ToString(), x.IsManaged.ToString(), x.Status };
            return string.Join("\t", f.Select(s => Convert.ToBase64String(Encoding.UTF8.GetBytes(s ?? ""))));
        }

        private static StartupItem CacheDecode(string line)
        {
            try
            {
                var f = line.Split('\t').Select(s => Encoding.UTF8.GetString(Convert.FromBase64String(s))).ToArray();
                if (f.Length < 11) return null;
                return new StartupItem { Id = f[0], Name = f[1], Source = f[2], Scope = f[3], Command = f[4], Location = f[5], AppName = f[6], Enabled = bool.Parse(f[7]), CanDisable = bool.Parse(f[8]), IsManaged = bool.Parse(f[9]), Status = f[10] };
            }
            catch { return null; }
        }

        private void BuildTray()
        {
            _tray = new FormsNotifyIcon { Icon = Program.AppIcon, Text = "Mich Startup Master", Visible = true };
            _tray.DoubleClick += (s, e) => ShowFromTray();
            _tray.ContextMenuStrip = new FormsContextMenuStrip();
            _tray.ContextMenuStrip.Items.Add("Open", null, (s, e) => ShowFromTray());
            _tray.ContextMenuStrip.Items.Add("Refresh", null, async (s, e) => await RefreshInventoryAsync(true));
            _tray.ContextMenuStrip.Items.Add("Enforce guards", null, async (s, e) => await RunGuardsAsync(true));
            _tray.ContextMenuStrip.Items.Add("Exit", null, (s, e) => { _reallyExit = true; Close(); });
        }

        private void HideToTray()
        {
            ShowInTaskbar = false;
            Hide();
        }

        private void ShowFromTray()
        {
            Dispatcher.Invoke(() =>
            {
                ShowInTaskbar = true;
                Show();
                WindowState = WindowState.Normal;
                Activate();
            });
        }

        private void OnClosing(object sender, CancelEventArgs e)
        {
            if (!_reallyExit)
            {
                e.Cancel = true;
                HideToTray();
                return;
            }
            if (_tray != null)
            {
                _tray.Visible = false;
                _tray.Dispose();
            }
        }

        private static ImageSource IconSource()
        {
            try
            {
                return Imaging.CreateBitmapSourceFromHIcon(Program.AppIcon.Handle, Int32Rect.Empty, BitmapSizeOptions.FromEmptyOptions());
            }
            catch { return null; }
        }

        private static SolidColorBrush BrushOf(string hex)
        {
            return (SolidColorBrush)new BrushConverter().ConvertFromString(hex);
        }
    }

    internal sealed class StartupRow
    {
        public StartupItem Item { get; private set; }
        public bool Enabled { get { return Item.Enabled; } }
        public bool CanDisable { get { return Item.CanDisable; } }
        public bool IsManaged { get { return Item.IsManaged; } }
        public string Status { get { return Item.Enabled ? "Enabled" : "Disabled"; } }
        public string Application { get { return Item.HumanName(); } }
        public string Entry { get { return Item.Name; } }
        public string Source { get { return Item.Source; } }
        public string Risk { get { return Item.RiskLabel(); } }
        public string Cleanup { get { return Item.AdviceLevel() == "Cleanup" ? "Remove?" : "Keep"; } }
        public string Popup { get { return Item.PopupLabel(); } }
        public string Location { get { return Item.Location; } }
        public string Command { get { return Item.Command; } }
        public Brush PopupBrush { get { return Popup == "Enabled" ? StartupWindowBrushes.Warn : (Popup == "Disabled" ? StartupWindowBrushes.Good : StartupWindowBrushes.Muted); } }
        public string SearchText { get { return (Application + " " + Entry + " " + Source + " " + Risk + " " + Cleanup + " " + Popup + " " + Location + " " + Command + " " + Item.Status); } }
        public StartupRow(StartupItem item) { Item = item; }
    }

    internal static class StartupWindowBrushes
    {
        public static readonly Brush Warn = (Brush)new BrushConverter().ConvertFromString("#f59e0b");
        public static readonly Brush Good = (Brush)new BrushConverter().ConvertFromString("#34d399");
        public static readonly Brush Muted = (Brush)new BrushConverter().ConvertFromString("#64748b");
    }

    internal sealed class StartupEditWindow : Window
    {
        private TextBox _name;
        private TextBox _path;
        private TextBox _args;
        private RadioButton _normal;
        private RadioButton _tray;

        public string AppTitle { get { return _name.Text.Trim(); } }
        public string AppPath { get { return _path.Text.Trim().Trim('"'); } }
        public string AppArguments { get { return _args.Text; } }
        public bool TrayMode { get { return _tray.IsChecked == true; } }

        public StartupEditWindow() : this("", "", "", true) { }

        public StartupEditWindow(string title, string path, string args, bool trayMode)
        {
            Title = string.IsNullOrWhiteSpace(path) ? "Add Startup App" : "Edit Startup App";
            Width = 720;
            Height = 510;
            ResizeMode = ResizeMode.NoResize;
            WindowStartupLocation = WindowStartupLocation.CenterOwner;
            Background = (Brush)new BrushConverter().ConvertFromString("#0a111c");
            Foreground = Brushes.White;
            Icon = Imaging.CreateBitmapSourceFromHIcon(Program.AppIcon.Handle, Int32Rect.Empty, BitmapSizeOptions.FromEmptyOptions());
            var root = new Grid { Margin = new Thickness(24) };
            Content = root;
            for (int i = 0; i < 8; i++) root.RowDefinitions.Add(new RowDefinition { Height = i == 7 ? new GridLength(1, GridUnitType.Star) : GridLength.Auto });
            root.Children.Add(new TextBlock { Text = Title, FontSize = 26, FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 0, 0, 8) });
            AddLabel(root, "Friendly name", 1);
            _name = Box(root, 2);
            AddLabel(root, "Executable, script, command, or shortcut", 3);
            var pathGrid = new Grid { Margin = new Thickness(0, 6, 0, 14) };
            pathGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            pathGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            pathGrid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            Grid.SetRow(pathGrid, 4);
            root.Children.Add(pathGrid);
            _path = new TextBox { Height = 34, Background = (Brush)new BrushConverter().ConvertFromString("#101a28"), Foreground = Brushes.White, BorderBrush = Brushes.DimGray, Padding = new Thickness(8, 5, 8, 5), Text = path ?? "" };
            pathGrid.Children.Add(_path);
            var browse = SmallButton("Browse");
            browse.Click += Browse;
            Grid.SetColumn(browse, 1);
            pathGrid.Children.Add(browse);
            var paste = SmallButton("Paste");
            paste.Click += Paste;
            Grid.SetColumn(paste, 2);
            pathGrid.Children.Add(paste);
            AddLabel(root, "Optional arguments", 5);
            _args = Box(root, 6);
            var mode = new StackPanel { Margin = new Thickness(0, 12, 0, 0) };
            Grid.SetRow(mode, 7);
            root.Children.Add(mode);
            _normal = new RadioButton { Content = "Start normally: run the target directly at Windows logon", Foreground = Brushes.White, Margin = new Thickness(0, 5, 0, 5), IsChecked = !trayMode };
            _tray = new RadioButton { Content = "Start quietly: use Startup Master tray wrapper when possible", Foreground = Brushes.White, Margin = new Thickness(0, 5, 0, 12), IsChecked = trayMode };
            mode.Children.Add(_normal);
            mode.Children.Add(_tray);
            var buttons = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right };
            mode.Children.Add(buttons);
            var ok = SmallButton("Save Startup");
            ok.MinWidth = 130;
            ok.Click += Save;
            buttons.Children.Add(ok);
            var cancel = SmallButton("Cancel");
            cancel.Click += (s, e) => { DialogResult = false; Close(); };
            buttons.Children.Add(cancel);
            _name.Text = title ?? "";
            _args.Text = args ?? "";
        }

        private void AddLabel(Grid root, string text, int row)
        {
            var label = new TextBlock { Text = text, Foreground = Brushes.LightSlateGray, FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 10, 0, 0) };
            Grid.SetRow(label, row);
            root.Children.Add(label);
        }

        private TextBox Box(Grid root, int row)
        {
            var box = new TextBox { Height = 34, Margin = new Thickness(0, 6, 0, 4), Background = (Brush)new BrushConverter().ConvertFromString("#101a28"), Foreground = Brushes.White, BorderBrush = Brushes.DimGray, Padding = new Thickness(8, 5, 8, 5) };
            Grid.SetRow(box, row);
            root.Children.Add(box);
            return box;
        }

        private Button SmallButton(string text)
        {
            return new Button { Content = text, Height = 34, MinWidth = 86, Margin = new Thickness(8, 0, 0, 0), Background = (Brush)new BrushConverter().ConvertFromString("#172436"), Foreground = Brushes.White, BorderBrush = Brushes.DimGray, Padding = new Thickness(12, 0, 12, 0), FontWeight = FontWeights.SemiBold };
        }

        private void Browse(object sender, RoutedEventArgs e)
        {
            var dlg = new Microsoft.Win32.OpenFileDialog { Filter = "Startup targets (*.exe;*.cmd;*.bat;*.ps1;*.lnk)|*.exe;*.cmd;*.bat;*.ps1;*.lnk|All files (*.*)|*.*", Title = "Choose startup target" };
            if (dlg.ShowDialog(this) == true)
            {
                _path.Text = dlg.FileName;
                if (string.IsNullOrWhiteSpace(_name.Text)) _name.Text = Path.GetFileNameWithoutExtension(dlg.FileName);
            }
        }

        private void Paste(object sender, RoutedEventArgs e)
        {
            string text = System.Windows.Clipboard.ContainsText() ? System.Windows.Clipboard.GetText().Trim().Trim('"') : "";
            if (File.Exists(text))
            {
                _path.Text = text;
                if (string.IsNullOrWhiteSpace(_name.Text)) _name.Text = Path.GetFileNameWithoutExtension(text);
            }
            else System.Windows.MessageBox.Show("Clipboard does not contain an existing startup target path.", "Paste path", MessageBoxButton.OK, MessageBoxImage.Information);
        }

        private void Save(object sender, RoutedEventArgs e)
        {
            if (string.IsNullOrWhiteSpace(AppTitle) || string.IsNullOrWhiteSpace(AppPath) || !File.Exists(AppPath) || !StartupService.IsSupportedStartupTarget(AppPath))
            {
                System.Windows.MessageBox.Show("Choose a valid .exe, .cmd, .bat, .ps1, or .lnk file and a friendly name.", "Missing startup target", MessageBoxButton.OK, MessageBoxImage.Warning);
                return;
            }
            DialogResult = true;
            Close();
        }
    }
}
