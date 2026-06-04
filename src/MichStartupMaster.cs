using Microsoft.Win32;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.IO;
using System.Linq;
using System.Management;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using System.Windows.Forms;

namespace MichStartupMaster
{
    internal static class Program
    {
        public static readonly string AppName = "MichStartupMaster";
        public static readonly string AppData = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), AppName);
        public static readonly string DisabledStore = Path.Combine(AppData, "disabled-items.tsv");
        public static readonly string DisabledStartupFolder = Path.Combine(AppData, "DisabledStartupFolderItems");
        public static readonly string ManagedTaskRoot = @"\MichStartupMaster\";

        [STAThread]
        private static int Main(string[] args)
        {
            Directory.CreateDirectory(AppData);
            if (args.Length > 0)
            {
                string cmd = args[0].ToLowerInvariant();
                if (cmd == "--smoke") return Smoke();
                if (cmd == "--list") { Console.WriteLine(StartupService.ToJson(StartupService.ScanAll())); return 0; }
                if (cmd == "--add-test-task") return CliAddTestTask(args);
                if (cmd == "--remove-task") return CliRemoveTask(args);
                if (cmd == "--tray-run") { TrayRunner.Run(args.Skip(1).ToArray()); return 0; }
            }

            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            Application.Run(new MainForm());
            return 0;
        }

        private static int Smoke()
        {
            var items = StartupService.ScanAll();
            Console.WriteLine("SMOKE OK inventory=" + items.Count + " user=" + Environment.UserName + " appdata=" + AppData);
            return items.Count >= 0 ? 0 : 1;
        }

        private static int CliAddTestTask(string[] args)
        {
            string exe = Process.GetCurrentProcess().MainModule.FileName;
            string name = "HermesSmoke-" + DateTime.Now.ToString("yyyyMMddHHmmss");
            string target = args.Length > 1 ? args[1] : exe;
            StartupService.AddManagedStartup(name, target, "--smoke", true, true);
            bool exists = StartupService.ScanAll().Any(x => x.Name.EndsWith(name, StringComparison.OrdinalIgnoreCase) && x.Source == "Scheduled Task" && x.Enabled);
            Console.WriteLine("ADD_TEST_TASK " + name + " exists=" + exists);
            return exists ? 0 : 2;
        }

        private static int CliRemoveTask(string[] args)
        {
            if (args.Length < 2) { Console.WriteLine("missing task name"); return 2; }
            StartupService.DeleteManagedTask(args[1]);
            bool exists = StartupService.ScanAll().Any(x => x.Name == args[1]);
            Console.WriteLine("REMOVE_TASK " + args[1] + " exists=" + exists);
            return exists ? 3 : 0;
        }
    }

    internal sealed class StartupItem
    {
        public string Id;
        public string Name;
        public string Source;
        public string Scope;
        public string Command;
        public string Location;
        public bool Enabled;
        public bool CanDisable;
        public bool IsManaged;
        public string Status;

        public string RiskLabel()
        {
            string c = (Command ?? "").ToLowerInvariant();
            if (c.Contains("temp") || c.Contains("appdata\\local\\temp") || c.Contains("powershell") || c.Contains("cmd.exe")) return "Review";
            if (Scope == "Machine") return "System";
            return "Normal";
        }
    }

    internal static class StartupService
    {
        public static List<StartupItem> ScanAll()
        {
            var items = new List<StartupItem>();
            AddWmiStartupCommands(items);
            AddRegistryRun(items, Registry.CurrentUser, @"Software\Microsoft\Windows\CurrentVersion\Run", "User");
            AddRegistryRun(items, Registry.LocalMachine, @"Software\Microsoft\Windows\CurrentVersion\Run", "Machine");
            AddStartupFolder(items, Environment.GetFolderPath(Environment.SpecialFolder.Startup), "User");
            AddStartupFolder(items, Environment.GetFolderPath(Environment.SpecialFolder.CommonStartup), "Machine");
            AddLogonTasks(items);
            items.AddRange(DisabledStoreService.LoadDisabledItems());
            return Dedupe(items).OrderBy(x => x.Enabled ? 0 : 1).ThenBy(x => x.Source).ThenBy(x => x.Name).ToList();
        }

        private static List<StartupItem> Dedupe(List<StartupItem> items)
        {
            var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var result = new List<StartupItem>();
            foreach (var item in items)
            {
                string key = (item.Source + "|" + item.Location + "|" + item.Name + "|" + item.Command).ToLowerInvariant();
                if (seen.Add(key)) result.Add(item);
            }
            return result;
        }

        private static void AddWmiStartupCommands(List<StartupItem> items)
        {
            try
            {
                using (var searcher = new ManagementObjectSearcher("SELECT Name, Command, Location, User, UserSID FROM Win32_StartupCommand"))
                {
                    foreach (ManagementObject mo in searcher.Get())
                    {
                        string name = Convert.ToString(mo["Name"] ?? "(startup)");
                        string command = Convert.ToString(mo["Command"] ?? "");
                        string location = Convert.ToString(mo["Location"] ?? "");
                        string user = Convert.ToString(mo["User"] ?? "");
                        string sid = Convert.ToString(mo["UserSID"] ?? "");
                        string scope = string.IsNullOrWhiteSpace(user) ? "Machine/User" : user;
                        items.Add(new StartupItem { Id = "wmi|" + Convert.ToBase64String(Encoding.UTF8.GetBytes(location + "|" + name)), Name = name, Source = "Startup Command", Scope = scope, Command = command, Location = location, Enabled = true, CanDisable = false, IsManaged = false, Status = "Discovered by Win32_StartupCommand" + (string.IsNullOrWhiteSpace(sid) ? "" : " (" + sid + ")") });
                    }
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("Startup Command", "System", "Win32_StartupCommand", ex)); }
        }

        private static void AddRegistryRun(List<StartupItem> items, RegistryKey root, string subKey, string scope)
        {
            try
            {
                using (var key = root.OpenSubKey(subKey, false))
                {
                    if (key == null) return;
                    foreach (var name in key.GetValueNames())
                    {
                        var value = key.GetValue(name);
                        string cmd = value == null ? "" : value.ToString();
                        string encodedName = Convert.ToBase64String(Encoding.UTF8.GetBytes(name ?? ""));
                        items.Add(new StartupItem { Id = "reg|" + scope + "|" + encodedName, Name = string.IsNullOrWhiteSpace(name) ? "(Default)" : name, Source = "Registry Run", Scope = scope, Command = cmd, Location = root.Name + @"\" + subKey, Enabled = true, CanDisable = scope == "User" || IsElevated(), IsManaged = false, Status = "Runs immediately from Run key" });
                    }
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("Registry Run", scope, root.Name + @"\" + subKey, ex)); }
        }

        private static void AddStartupFolder(List<StartupItem> items, string folder, string scope)
        {
            try
            {
                if (!Directory.Exists(folder)) return;
                foreach (var file in Directory.GetFiles(folder))
                {
                    items.Add(new StartupItem { Id = "folder|" + scope + "|" + file, Name = Path.GetFileName(file), Source = "Startup Folder", Scope = scope, Command = file, Location = folder, Enabled = true, CanDisable = scope == "User" || IsElevated(), IsManaged = false, Status = "Starts through Startup folder" });
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("Startup Folder", scope, folder, ex)); }
        }

        private static void AddLogonTasks(List<StartupItem> items)
        {
            try
            {
                string script = @"
$ErrorActionPreference='Stop'
foreach($t in Get-ScheduledTask){
  $hasLogon=$false; $hasDelay=$false
  foreach($tr in @($t.Triggers)){
    if($null -eq $tr){ continue }
    $cn = if($tr.CimClass){ [string]$tr.CimClass.CimClassName } else { '' }
    if($cn -like '*LogonTrigger*'){ $hasLogon=$true }
    $delayProp=$tr.PSObject.Properties['Delay']
    if($delayProp -and $delayProp.Value){ $hasDelay=$true }
  }
  if($hasLogon){
    $actions = (@($t.Actions) | ForEach-Object { if($_){ (($_.Execute) + ' ' + ($_.Arguments)).Trim() } }) -join ' || '
    $enabled = if($t.Settings.Enabled){'true'}else{'false'}
    $managed = if(($t.TaskPath + $t.TaskName).StartsWith('\MichStartupMaster\')){'true'}else{'false'}
    ($t.TaskPath + $t.TaskName) + ""`t"" + $enabled + ""`t"" + $t.State + ""`t"" + $hasDelay + ""`t"" + $managed + ""`t"" + $actions
  }
}
";
                string encoded = Convert.ToBase64String(Encoding.Unicode.GetBytes(script));
                string output = RunCapture("powershell.exe", "-NoProfile -ExecutionPolicy Bypass -EncodedCommand " + encoded);
                foreach (var line in output.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries))
                {
                    string[] p = line.Split(new[] { '\t' }, 6);
                    if (p.Length < 6) continue;
                    string taskName = p[0];
                    bool enabled = p[1].Equals("true", StringComparison.OrdinalIgnoreCase);
                    bool hasDelay = p[3].Equals("true", StringComparison.OrdinalIgnoreCase);
                    bool managed = p[4].Equals("true", StringComparison.OrdinalIgnoreCase);
                    string status = (enabled ? "Enabled" : "Disabled") + " logon task" + (hasDelay ? " with delay" : " with no delay");
                    items.Add(new StartupItem { Id = "task|" + taskName, Name = taskName.TrimStart('\\'), Source = "Scheduled Task", Scope = "User/System", Command = p[5], Location = taskName, Enabled = enabled, CanDisable = true, IsManaged = managed, Status = status });
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("Scheduled Task", "System", "Task Scheduler", ex)); }
        }

        public static void Disable(StartupItem item)
        {
            if (item.Id.StartsWith("reg|")) DisableRegistry(item);
            else if (item.Id.StartsWith("folder|")) DisableStartupFolder(item);
            else if (item.Id.StartsWith("task|")) RunChecked("schtasks.exe", "/Change /TN " + Q(item.Location) + " /Disable");
            else throw new InvalidOperationException("Unsupported item: " + item.Id);
        }

        public static void Enable(StartupItem item)
        {
            if (item.Id.StartsWith("disabled|reg|")) RestoreRegistry(item);
            else if (item.Id.StartsWith("disabled|folder|")) RestoreStartupFolder(item);
            else if (item.Id.StartsWith("task|")) RunChecked("schtasks.exe", "/Change /TN " + Q(item.Location) + " /Enable");
            else throw new InvalidOperationException("Unsupported disabled item: " + item.Id);
        }

        private static void DisableRegistry(StartupItem item)
        {
            string[] p = item.Id.Split('|');
            string scope = p[1]; string name = Encoding.UTF8.GetString(Convert.FromBase64String(p[2]));
            RegistryKey root = scope == "Machine" ? Registry.LocalMachine : Registry.CurrentUser;
            using (var key = root.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run", true))
            {
                if (key == null) throw new InvalidOperationException("Run key missing");
                object value = key.GetValue(name, null, RegistryValueOptions.DoNotExpandEnvironmentNames);
                RegistryValueKind kind = key.GetValueKind(name);
                DisabledStoreService.Add("reg", item.Name, scope, value == null ? "" : value.ToString(), item.Location, name + "\t" + kind.ToString());
                key.DeleteValue(name, false);
            }
        }

        private static void RestoreRegistry(StartupItem item)
        {
            string[] meta = (item.Status ?? "").Split('\t');
            string valueName = meta.Length > 0 ? meta[0] : item.Name;
            RegistryKey root = item.Scope == "Machine" ? Registry.LocalMachine : Registry.CurrentUser;
            RegistryValueKind kind = RegistryValueKind.String;
            if (meta.Length > 1) { try { kind = (RegistryValueKind)Enum.Parse(typeof(RegistryValueKind), meta[1], true); } catch { kind = RegistryValueKind.String; } }
            using (var key = root.CreateSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run")) key.SetValue(valueName, item.Command ?? "", kind);
            DisabledStoreService.Remove(item.Id);
        }

        private static void DisableStartupFolder(StartupItem item)
        {
            Directory.CreateDirectory(Program.DisabledStartupFolder);
            string source = item.Command;
            string dest = Path.Combine(Program.DisabledStartupFolder, Path.GetFileName(source) + "." + DateTime.Now.Ticks + ".disabled");
            File.Move(source, dest);
            DisabledStoreService.Add("folder", item.Name, item.Scope, dest, item.Location, source);
        }

        private static void RestoreStartupFolder(StartupItem item)
        {
            string original = item.Status;
            string disabledPath = item.Command;
            Directory.CreateDirectory(Path.GetDirectoryName(original));
            File.Move(disabledPath, original);
            DisabledStoreService.Remove(item.Id);
        }

        public static void AddManagedStartup(string name, string targetPath, string arguments, bool trayMode, bool noDelay)
        {
            if (string.IsNullOrWhiteSpace(name)) throw new ArgumentException("Name is required");
            if (string.IsNullOrWhiteSpace(targetPath) || !File.Exists(targetPath)) throw new FileNotFoundException("Application not found", targetPath);
            string safeName = Regex.Replace(name, "[^A-Za-z0-9 _.-]", "").Trim();
            if (safeName.Length == 0) safeName = "StartupApp";
            string execute;
            string actionArgs;
            if (trayMode)
            {
                execute = Process.GetCurrentProcess().MainModule.FileName;
                string payload = Convert.ToBase64String(Encoding.UTF8.GetBytes(targetPath + "\n" + (arguments ?? "")));
                actionArgs = "--tray-run " + payload;
            }
            else { execute = targetPath; actionArgs = arguments ?? ""; }
            RegisterLogonTask(safeName, execute, actionArgs);
            if (!noDelay) { /* Task Scheduler has no explicit Delay either way; this app always uses immediate logon triggers. */ }
        }

        private static void RegisterLogonTask(string taskName, string execute, string arguments)
        {
            string script =
                "$ErrorActionPreference='Stop';" +
                "$path='\\MichStartupMaster\\';" +
                "$action=New-ScheduledTaskAction -Execute '" + PsSingle(execute) + "'" + (string.IsNullOrWhiteSpace(arguments) ? ";" : " -Argument '" + PsSingle(arguments) + "';") +
                "$trigger=New-ScheduledTaskTrigger -AtLogOn;" +
                "$principal=New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited;" +
                "$settings=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 0);" +
                "Register-ScheduledTask -TaskPath $path -TaskName '" + PsSingle(taskName) + "' -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null;";
            string encoded = Convert.ToBase64String(Encoding.Unicode.GetBytes(script));
            RunChecked("powershell.exe", "-NoProfile -ExecutionPolicy Bypass -EncodedCommand " + encoded);
        }

        private static string PsSingle(string value) { return (value ?? "").Replace("'", "''"); }

        public static void DeleteManagedTask(string nameOrTask)
        {
            string tn = nameOrTask.StartsWith("\\") ? nameOrTask : Program.ManagedTaskRoot + nameOrTask.Replace(Program.ManagedTaskRoot.Trim('\\'), "").Trim('\\');
            RunChecked("schtasks.exe", "/Delete /F /TN " + Q(tn));
        }

        public static string ToJson(List<StartupItem> items)
        {
            var sb = new StringBuilder(); sb.Append("[");
            for (int i = 0; i < items.Count; i++)
            {
                if (i > 0) sb.Append(','); var x = items[i];
                sb.Append("{\"name\":\"").Append(Esc(x.Name)).Append("\",\"source\":\"").Append(Esc(x.Source)).Append("\",\"enabled\":").Append(x.Enabled ? "true" : "false").Append(",\"command\":\"").Append(Esc(x.Command)).Append("\"}");
            }
            sb.Append("]"); return sb.ToString();
        }

        private static string Esc(string s) { return (s ?? "").Replace("\\", "\\\\").Replace("\"", "\\\"").Replace("\r", " ").Replace("\n", " "); }
        private static StartupItem ErrorItem(string source, string scope, string location, Exception ex) { return new StartupItem { Id = "error|" + source + "|" + location, Name = "Scan warning", Source = source, Scope = scope, Location = location, Command = ex.Message, Enabled = false, CanDisable = false, Status = "Read failed" }; }
        private static string Q(string s) { return "\"" + (s ?? "").Replace("\"", "\\\"") + "\""; }

        private static bool IsElevated()
        {
            try { var id = System.Security.Principal.WindowsIdentity.GetCurrent(); var p = new System.Security.Principal.WindowsPrincipal(id); return p.IsInRole(System.Security.Principal.WindowsBuiltInRole.Administrator); } catch { return false; }
        }

        private static string RunCapture(string exe, string args)
        {
            var psi = new ProcessStartInfo(exe, args) { UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true, CreateNoWindow = true };
            using (var p = Process.Start(psi)) { string o = p.StandardOutput.ReadToEnd(); string e = p.StandardError.ReadToEnd(); p.WaitForExit(20000); if (p.ExitCode != 0 && string.IsNullOrWhiteSpace(o)) throw new Exception(e); return o; }
        }

        private static void RunChecked(string exe, string args)
        {
            var psi = new ProcessStartInfo(exe, args) { UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true, CreateNoWindow = true };
            using (var p = Process.Start(psi)) { string o = p.StandardOutput.ReadToEnd(); string e = p.StandardError.ReadToEnd(); p.WaitForExit(20000); if (p.ExitCode != 0) throw new Exception((o + " " + e).Trim()); }
        }

        private static IEnumerable<Dictionary<string, string>> CsvRows(string csv)
        {
            var lines = csv.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries); if (lines.Length < 2) yield break;
            var headers = CsvParseLine(lines[0]);
            for (int i = 1; i < lines.Length; i++) { var vals = CsvParseLine(lines[i]); var d = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase); for (int j = 0; j < headers.Count && j < vals.Count; j++) d[headers[j]] = vals[j]; yield return d; }
        }
        private static List<string> CsvParseLine(string line)
        {
            var r = new List<string>(); var sb = new StringBuilder(); bool q = false;
            for (int i = 0; i < line.Length; i++) { char c = line[i]; if (c == '"') { if (q && i + 1 < line.Length && line[i + 1] == '"') { sb.Append('"'); i++; } else q = !q; } else if (c == ',' && !q) { r.Add(sb.ToString()); sb.Clear(); } else sb.Append(c); }
            r.Add(sb.ToString()); return r;
        }
    }

    internal static class DisabledStoreService
    {
        public static void Add(string type, string name, string scope, string command, string location, string status)
        {
            Directory.CreateDirectory(Program.AppData);
            string id = "disabled|" + type + "|" + Guid.NewGuid().ToString("N");
            File.AppendAllText(Program.DisabledStore, string.Join("\t", new[] { id, type, B64(name), B64(scope), B64(command), B64(location), B64(status) }) + Environment.NewLine, Encoding.UTF8);
        }
        public static List<StartupItem> LoadDisabledItems()
        {
            var list = new List<StartupItem>(); if (!File.Exists(Program.DisabledStore)) return list;
            foreach (var line in File.ReadAllLines(Program.DisabledStore))
            {
                var p = line.Split('\t'); if (p.Length < 7) continue;
                list.Add(new StartupItem { Id = p[0], Name = UnB64(p[2]), Source = p[1] == "reg" ? "Registry Run" : "Startup Folder", Scope = UnB64(p[3]), Command = UnB64(p[4]), Location = UnB64(p[5]), Status = UnB64(p[6]), Enabled = false, CanDisable = true, IsManaged = true });
            }
            return list;
        }
        public static void Remove(string id)
        {
            if (!File.Exists(Program.DisabledStore)) return;
            var kept = File.ReadAllLines(Program.DisabledStore).Where(l => !l.StartsWith(id + "\t", StringComparison.Ordinal)).ToArray();
            File.WriteAllLines(Program.DisabledStore, kept, Encoding.UTF8);
        }
        private static string B64(string s) { return Convert.ToBase64String(Encoding.UTF8.GetBytes(s ?? "")); }
        private static string UnB64(string s) { try { return Encoding.UTF8.GetString(Convert.FromBase64String(s)); } catch { return ""; } }
    }

    internal static class TrayRunner
    {
        public static void Run(string[] args)
        {
            if (args.Length < 1) return;
            string decoded = Encoding.UTF8.GetString(Convert.FromBase64String(args[0]));
            string[] lines = decoded.Split(new[] { '\n' }, 2);
            string target = lines[0]; string targetArgs = lines.Length > 1 ? lines[1] : "";
            Application.EnableVisualStyles();
            var ctx = new ApplicationContext();
            var icon = new NotifyIcon();
            icon.Icon = SystemIcons.Application;
            icon.Text = "Mich Startup Master: " + Path.GetFileName(target);
            icon.Visible = true;
            icon.ContextMenu = new ContextMenu(new[] { new MenuItem("Open manager", (s, e) => Process.Start(Process.GetCurrentProcess().MainModule.FileName)), new MenuItem("Exit tray wrapper", (s, e) => { icon.Visible = false; icon.Dispose(); ctx.ExitThread(); }) });
            try
            {
                var psi = new ProcessStartInfo(target, targetArgs) { UseShellExecute = true, WindowStyle = ProcessWindowStyle.Minimized };
                Process.Start(psi);
                icon.ShowBalloonTip(2500, "Startup launched quietly", Path.GetFileName(target) + " was started minimized from tray mode.", ToolTipIcon.Info);
            }
            catch (Exception ex) { icon.ShowBalloonTip(5000, "Startup launch failed", ex.Message, ToolTipIcon.Error); }
            Application.Run(ctx);
        }
    }

    internal sealed class MainForm : Form
    {
        private List<StartupItem> _items = new List<StartupItem>();
        private ListView _list; private TextBox _search; private Label _summary; private Button _refresh; private Button _disable; private Button _enable; private Button _add; private Button _deleteManaged; private NotifyIcon _tray;
        private readonly Color Bg = Color.FromArgb(10, 14, 28), Panel = Color.FromArgb(21, 28, 51), Panel2 = Color.FromArgb(29, 39, 70), Accent = Color.FromArgb(99, 102, 241), TextMain = Color.FromArgb(245, 247, 255), Muted = Color.FromArgb(156, 166, 195), Good = Color.FromArgb(52, 211, 153), Danger = Color.FromArgb(248, 113, 113);
        public MainForm()
        {
            Text = "Mich Startup Master — Windows Boot Control"; Width = 1180; Height = 760; MinimumSize = new Size(960, 620); BackColor = Bg; Font = new Font("Segoe UI", 10f); DoubleBuffered = true; Icon = SystemIcons.Shield;
            BuildUi(); BuildTray(); Load += (s, e) => RefreshItems(); FormClosing += OnClosingToTray;
        }
        protected override void OnPaint(PaintEventArgs e)
        {
            using (var b = new LinearGradientBrush(ClientRectangle, Color.FromArgb(8, 12, 26), Color.FromArgb(30, 18, 62), 35f)) e.Graphics.FillRectangle(b, ClientRectangle);
            using (var pen = new Pen(Color.FromArgb(40, 255, 255, 255))) e.Graphics.DrawLine(pen, 32, 132, Width - 48, 132);
            base.OnPaint(e);
        }
        private void BuildUi()
        {
            var title = new Label { Text = "Every Windows boot item, one beautiful control room", ForeColor = TextMain, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 23f), AutoSize = true, Location = new Point(32, 24) };
            var sub = new Label { Text = "Disable registry Run keys, Startup-folder entries, and logon scheduled tasks. Add zero-delay normal or tray-mode launches.", ForeColor = Muted, BackColor = Color.Transparent, Font = new Font("Segoe UI", 11f), AutoSize = true, Location = new Point(36, 72) };
            _summary = new Label { ForeColor = Color.White, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 11f), AutoSize = true, Location = new Point(36, 107) };
            Controls.Add(title); Controls.Add(sub); Controls.Add(_summary);
            _search = StyledTextBox("Search name, command, source..."); _search.Location = new Point(32, 150); _search.Width = 390; _search.TextChanged += (s, e) => RenderList(); Controls.Add(_search);
            _refresh = Button("Refresh", Accent); _refresh.Location = new Point(438, 148); _refresh.Click += (s, e) => RefreshItems(); Controls.Add(_refresh);
            _disable = Button("Disable selected", Danger); _disable.Location = new Point(548, 148); _disable.Click += (s, e) => DisableSelected(); Controls.Add(_disable);
            _enable = Button("Enable selected", Good); _enable.Location = new Point(704, 148); _enable.Click += (s, e) => EnableSelected(); Controls.Add(_enable);
            _add = Button("+ Add boot app", Accent); _add.Location = new Point(850, 148); _add.Click += (s, e) => AddBootApp(); Controls.Add(_add);
            _deleteManaged = Button("Delete managed", Color.FromArgb(234, 179, 8)); _deleteManaged.Location = new Point(994, 148); _deleteManaged.Click += (s, e) => DeleteManaged(); Controls.Add(_deleteManaged);
            _list = new ListView { Location = new Point(32, 204), Size = new Size(Width - 82, Height - 270), Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right | AnchorStyles.Bottom, View = View.Details, FullRowSelect = true, GridLines = false, BorderStyle = BorderStyle.None, BackColor = Color.FromArgb(15, 21, 39), ForeColor = TextMain, Font = new Font("Segoe UI", 9.7f), HideSelection = false, OwnerDraw = true };
            _list.Columns.Add("State", 85); _list.Columns.Add("Name", 230); _list.Columns.Add("Source", 140); _list.Columns.Add("Risk", 90); _list.Columns.Add("Command / target", 560); _list.Columns.Add("Location", 260);
            _list.DrawColumnHeader += (s, e) => { using (var b = new SolidBrush(Panel2)) e.Graphics.FillRectangle(b, e.Bounds); TextRenderer.DrawText(e.Graphics, e.Header.Text, new Font(Font, FontStyle.Bold), e.Bounds, Color.White, TextFormatFlags.VerticalCenter | TextFormatFlags.Left); };
            _list.DrawSubItem += DrawSubItem; _list.Resize += (s, e) => { if (_list.Columns.Count > 4) _list.Columns[4].Width = Math.Max(380, _list.Width - 845); };
            Controls.Add(_list);
        }
        private void DrawSubItem(object sender, DrawListViewSubItemEventArgs e)
        {
            var item = (StartupItem)e.Item.Tag; bool selected = e.Item.Selected;
            Color row = selected ? Color.FromArgb(52, 64, 116) : (e.ItemIndex % 2 == 0 ? Color.FromArgb(15, 21, 39) : Color.FromArgb(18, 25, 46));
            using (var b = new SolidBrush(row)) e.Graphics.FillRectangle(b, e.Bounds);
            Color c = e.ColumnIndex == 0 ? (item.Enabled ? Good : Danger) : (e.ColumnIndex == 3 && item.RiskLabel() == "Review" ? Color.FromArgb(251, 191, 36) : TextMain);
            string text = e.SubItem.Text;
            if (e.ColumnIndex == 0) text = item.Enabled ? "● Enabled" : "● Disabled";
            TextRenderer.DrawText(e.Graphics, text, _list.Font, new Rectangle(e.Bounds.X + 8, e.Bounds.Y, e.Bounds.Width - 10, e.Bounds.Height), c, TextFormatFlags.VerticalCenter | TextFormatFlags.EndEllipsis | TextFormatFlags.Left);
        }
        private TextBox StyledTextBox(string placeholder) { var t = new TextBox { BorderStyle = BorderStyle.FixedSingle, BackColor = Color.FromArgb(17, 24, 44), ForeColor = Color.White, Font = new Font("Segoe UI", 11f), Height = 32 }; return t; }
        private Button Button(string text, Color color)
        {
            var b = new Button { Text = text, Width = 130, Height = 38, FlatStyle = FlatStyle.Flat, BackColor = color, ForeColor = Color.White, Font = new Font("Segoe UI Semibold", 9.5f), Cursor = Cursors.Hand };
            b.FlatAppearance.BorderSize = 0; b.MouseEnter += (s, e) => b.BackColor = ControlPaint.Light(color, .12f); b.MouseLeave += (s, e) => b.BackColor = color; return b;
        }
        private void BuildTray()
        {
            _tray = new NotifyIcon { Icon = SystemIcons.Shield, Text = "Mich Startup Master", Visible = true };
            _tray.DoubleClick += (s, e) => { Show(); WindowState = FormWindowState.Normal; Activate(); };
            _tray.ContextMenu = new ContextMenu(new[] { new MenuItem("Open Startup Master", (s, e) => { Show(); WindowState = FormWindowState.Normal; Activate(); }), new MenuItem("Refresh inventory", (s, e) => RefreshItems()), new MenuItem("Exit", (s, e) => { _tray.Visible = false; _tray.Dispose(); Application.Exit(); }) });
        }
        private void OnClosingToTray(object sender, FormClosingEventArgs e) { if (_tray != null && _tray.Visible && e.CloseReason == CloseReason.UserClosing) { e.Cancel = true; Hide(); _tray.ShowBalloonTip(1800, "Still running", "Startup Master is in the system tray.", ToolTipIcon.Info); } }
        private void RefreshItems() { Cursor = Cursors.WaitCursor; try { _items = StartupService.ScanAll(); RenderList(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Refresh failed", MessageBoxButtons.OK, MessageBoxIcon.Error); } finally { Cursor = Cursors.Default; } }
        private void RenderList()
        {
            string q = (_search.Text ?? "").Trim().ToLowerInvariant();
            var rows = _items.Where(x => string.IsNullOrEmpty(q) || (x.Name + " " + x.Command + " " + x.Source).ToLowerInvariant().Contains(q)).ToList();
            _list.BeginUpdate(); _list.Items.Clear(); foreach (var x in rows) { var li = new ListViewItem(x.Enabled ? "Enabled" : "Disabled") { Tag = x }; li.SubItems.Add(x.Name); li.SubItems.Add(x.Source); li.SubItems.Add(x.RiskLabel()); li.SubItems.Add(x.Command); li.SubItems.Add(x.Location); _list.Items.Add(li); } _list.EndUpdate();
            _summary.Text = rows.Count + " visible • " + _items.Count(x => x.Enabled) + " enabled • " + _items.Count(x => !x.Enabled) + " disabled • " + _items.Count(x => x.IsManaged) + " managed";
        }
        private StartupItem Selected() { return _list.SelectedItems.Count == 0 ? null : (StartupItem)_list.SelectedItems[0].Tag; }
        private void DisableSelected() { var x = Selected(); if (x == null) return; try { StartupService.Disable(x); Toast("Disabled", x.Name + " will not run next boot."); RefreshItems(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Disable failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); } }
        private void EnableSelected() { var x = Selected(); if (x == null) return; try { StartupService.Enable(x); Toast("Enabled", x.Name + " will run next boot."); RefreshItems(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Enable failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); } }
        private void DeleteManaged() { var x = Selected(); if (x == null || !x.IsManaged || x.Source != "Scheduled Task") { MessageBox.Show("Select a MichStartupMaster managed scheduled task."); return; } try { StartupService.DeleteManagedTask(x.Location); RefreshItems(); Toast("Deleted", x.Name); } catch (Exception ex) { MessageBox.Show(ex.Message, "Delete failed"); } }
        private void AddBootApp()
        {
            using (var d = new AddStartupForm())
            {
                if (d.ShowDialog(this) != DialogResult.OK) return;
                try { StartupService.AddManagedStartup(d.AppTitle, d.AppPath, d.AppArguments, d.TrayMode, true); Toast("Added zero-delay boot app", d.AppTitle); RefreshItems(); }
                catch (Exception ex) { MessageBox.Show(ex.Message, "Add failed", MessageBoxButtons.OK, MessageBoxIcon.Error); }
            }
        }
        private void Toast(string title, string body) { if (_tray != null) _tray.ShowBalloonTip(2200, title, body, ToolTipIcon.Info); }
    }

    internal sealed class AddStartupForm : Form
    {
        public string AppTitle { get { return _name.Text.Trim(); } } public string AppPath { get { return _path.Text.Trim(); } } public string AppArguments { get { return _args.Text; } } public bool TrayMode { get { return _tray.Checked; } }
        private TextBox _name, _path, _args; private CheckBox _tray;
        public AddStartupForm()
        {
            Text = "Add zero-delay boot application"; Width = 640; Height = 360; FormBorderStyle = FormBorderStyle.FixedDialog; MaximizeBox = false; MinimizeBox = false; BackColor = Color.FromArgb(13, 18, 34); ForeColor = Color.White; Font = new Font("Segoe UI", 10f); StartPosition = FormStartPosition.CenterParent;
            AddLabel("Application name", 28); _name = Box(52); AddLabel("Executable path", 92); _path = Box(116); var browse = new Button { Text = "Browse", Left = 512, Top = 114, Width = 86, Height = 30 }; browse.Click += Browse; Controls.Add(browse); AddLabel("Arguments (optional)", 156); _args = Box(180);
            _tray = new CheckBox { Text = "Run through Startup Master tray wrapper: launch minimized and prevent a boot popup where Windows/app supports it", Left = 28, Top = 222, Width = 570, ForeColor = Color.FromArgb(196, 207, 235), Checked = true }; Controls.Add(_tray);
            var ok = new Button { Text = "Add at next boot", Left = 342, Top = 270, Width = 130, Height = 36, DialogResult = DialogResult.OK, BackColor = Color.FromArgb(99, 102, 241), ForeColor = Color.White, FlatStyle = FlatStyle.Flat }; var cancel = new Button { Text = "Cancel", Left = 486, Top = 270, Width = 90, Height = 36, DialogResult = DialogResult.Cancel }; Controls.Add(ok); Controls.Add(cancel); AcceptButton = ok; CancelButton = cancel;
        }
        private void AddLabel(string text, int top) { Controls.Add(new Label { Text = text, Left = 28, Top = top, AutoSize = true, ForeColor = Color.FromArgb(156, 166, 195) }); }
        private TextBox Box(int top) { var t = new TextBox { Left = 28, Top = top, Width = 470, Height = 28, BackColor = Color.FromArgb(24, 33, 58), ForeColor = Color.White, BorderStyle = BorderStyle.FixedSingle }; Controls.Add(t); return t; }
        private void Browse(object sender, EventArgs e) { using (var ofd = new OpenFileDialog { Filter = "Applications (*.exe)|*.exe|All files|*.*" }) if (ofd.ShowDialog(this) == DialogResult.OK) { _path.Text = ofd.FileName; if (string.IsNullOrWhiteSpace(_name.Text)) _name.Text = Path.GetFileNameWithoutExtension(ofd.FileName); } }
    }
}
