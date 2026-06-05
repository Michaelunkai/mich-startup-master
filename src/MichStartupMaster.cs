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
        public static readonly string AppUserModelId = "Mich.MichStartupMaster";
        public static readonly string AppData = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), AppName);

        [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern int SetCurrentProcessExplicitAppUserModelID(string appID);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern IntPtr FindWindow(string lpClassName, string lpWindowName);
        [DllImport("user32.dll")] private static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr hWnd);
        public static readonly string DisabledStore = Path.Combine(AppData, "disabled-items.tsv");
        public static readonly string ProtectedDisabledStore = Path.Combine(AppData, "protected-disabled-items.tsv");
        public static readonly string DisabledStartupFolder = Path.Combine(AppData, "DisabledStartupFolderItems");
        public static readonly string ManagedTaskRoot = @"\MichStartupMaster\";
        private static Icon _appIcon;

        public static Icon AppIcon
        {
            get
            {
                if (_appIcon != null) return _appIcon;
                try
                {
                    Icon extracted = Icon.ExtractAssociatedIcon(Process.GetCurrentProcess().MainModule.FileName);
                    if (extracted != null)
                    {
                        _appIcon = (Icon)extracted.Clone();
                        extracted.Dispose();
                        return _appIcon;
                    }
                }
                catch { }
                _appIcon = (Icon)SystemIcons.Shield.Clone();
                return _appIcon;
            }
        }

        [STAThread]
        private static int Main(string[] args)
        {
            SetCurrentProcessExplicitAppUserModelID(AppUserModelId);
            Directory.CreateDirectory(AppData);
            if (args.Length > 0)
            {
                string cmd = args[0].ToLowerInvariant();
                if (cmd == "--smoke") return Smoke();
                if (cmd == "--list") { Console.WriteLine(StartupService.ToJson(StartupService.ScanAll())); return 0; }
                if (cmd == "--add-test-task") return CliAddTestTask(args, true);
                if (cmd == "--add-test-task-tray") return CliAddTestTask(args, true);
                if (cmd == "--add-test-task-normal") return CliAddTestTask(args, false);
                if (cmd == "--remove-task") return CliRemoveTask(args);
                if (cmd == "--ui-contract") { Console.WriteLine(MainForm.UiContractJson()); return 0; }
                if (cmd == "--protect-disabled") { Console.WriteLine(ProtectedDisabledService.ProtectCurrentDisabled()); return 0; }
                if (cmd == "--enforce-disabled") { Console.WriteLine(ProtectedDisabledService.EnforceProtected()); return 0; }
                if (cmd == "--toggle-popup") return CliTogglePopup(args);
                if (cmd == "--tray-run") { TrayRunner.Run(args.Skip(1).ToArray()); return 0; }
                if (cmd == "--start-in-tray")
                {
                    Application.EnableVisualStyles();
                    Application.SetCompatibleTextRenderingDefault(false);
                    bool createdNew;
                    using (var singleInstance = new System.Threading.Mutex(true, @"Local\MichStartupMaster.MainInstance", out createdNew))
                    {
                        if (!createdNew) return 0;
                        Application.Run(new MainForm(true));
                    }
                    return 0;
                }
                if (cmd == "--show-add-dialog")
                {
                    Application.EnableVisualStyles();
                    Application.SetCompatibleTextRenderingDefault(false);
                    using (var dialog = new AddStartupForm()) dialog.ShowDialog();
                    return 0;
                }
            }

            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            bool createdMain;
            using (var singleInstance = new System.Threading.Mutex(true, @"Local\MichStartupMaster.MainInstance", out createdMain))
            {
                if (!createdMain)
                {
                    TryShowExistingMainWindow();
                    return 0;
                }
                Application.Run(new MainForm());
            }
            return 0;
        }

        private static void TryShowExistingMainWindow()
        {
            IntPtr h = FindWindow(null, "Mich Startup Master — Windows Boot Control");
            if (h == IntPtr.Zero) h = FindWindow(null, "Mich Startup Master - Windows Boot Control");
            if (h != IntPtr.Zero)
            {
                ShowWindowAsync(h, 9);
                SetForegroundWindow(h);
            }
        }

        private static int Smoke()
        {
            var items = StartupService.ScanAll();
            Console.WriteLine("SMOKE OK inventory=" + items.Count + " user=" + Environment.UserName + " appdata=" + AppData);
            return items.Count >= 0 ? 0 : 1;
        }

        private static int CliAddTestTask(string[] args, bool trayMode)
        {
            string exe = Process.GetCurrentProcess().MainModule.FileName;
            string name = (trayMode ? "HermesSmokeTray-" : "HermesSmokeNormal-") + DateTime.Now.ToString("yyyyMMddHHmmss");
            string target = args.Length > 1 ? args[1] : exe;
            StartupService.AddManagedStartup(name, target, "--smoke", trayMode, true);
            bool exists = StartupService.ScanAll().Any(x => x.Name.EndsWith(name, StringComparison.OrdinalIgnoreCase) && x.Source == "Scheduled Task" && x.Enabled);
            Console.WriteLine("ADD_TEST_TASK " + name + " mode=" + (trayMode ? "tray" : "normal") + " exists=" + exists);
            return exists ? 0 : 2;
        }

        private static int CliRemoveTask(string[] args)
        {
            if (args.Length < 2) { Console.WriteLine("missing task name"); return 2; }
            StartupService.DeleteManagedTask(args[1]);
            bool exists = StartupService.ScanAll().Any(x => x.Name == args[1] || x.Location == args[1]);
            Console.WriteLine("REMOVE_TASK " + args[1] + " exists=" + exists);
            return exists ? 3 : 0;
        }

        private static int CliTogglePopup(string[] args)
        {
            if (args.Length < 2) { Console.WriteLine("missing startup item name/location"); return 2; }
            string key = args[1];
            var item = StartupService.ScanAll().FirstOrDefault(x => x.Location.Equals(key, StringComparison.OrdinalIgnoreCase) || x.Name.Equals(key.TrimStart('\\'), StringComparison.OrdinalIgnoreCase));
            if (item == null) { Console.WriteLine("startup item not found: " + key); return 4; }
            StartupService.TogglePopupMode(item);
            var updated = StartupService.ScanAll().FirstOrDefault(x => x.Location.Equals(item.Location, StringComparison.OrdinalIgnoreCase) || x.Name.Equals(item.Name, StringComparison.OrdinalIgnoreCase));
            Console.WriteLine("TOGGLE_POPUP " + item.Location + " popup=" + (updated == null ? "changed" : updated.PopupLabel()));
            return 0;
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

        public string PopupLabel()
        {
            return StartupService.CommandUsesTrayWrapper(Command) ? "Disabled" : "Enabled";
        }

        public bool PopupEnabled()
        {
            return PopupLabel() == "Enabled";
        }

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
            ProtectedDisabledService.Protect(item);
        }

        public static void Enable(StartupItem item)
        {
            ProtectedDisabledService.Unprotect(item);
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
                if (IsSelfTarget(targetPath)) actionArgs = "--start-in-tray";
                else
                {
                    string payload = Convert.ToBase64String(Encoding.UTF8.GetBytes(targetPath + "\n" + (arguments ?? "")));
                    actionArgs = "--tray-run " + payload;
                }
            }
            else { execute = targetPath; actionArgs = arguments ?? ""; }
            RegisterLogonTaskAt(Program.ManagedTaskRoot + safeName, execute, actionArgs);
            if (!noDelay) { /* Task Scheduler has no explicit Delay either way; this app always uses immediate logon triggers. */ }
        }

        private static void RegisterLogonTaskAt(string fullTaskName, string execute, string arguments)
        {
            string normalized = fullTaskName.StartsWith("\\") ? fullTaskName : Program.ManagedTaskRoot + fullTaskName.Trim('\\');
            int split = normalized.LastIndexOf('\\');
            string path = split >= 0 ? normalized.Substring(0, split + 1) : Program.ManagedTaskRoot;
            string taskName = split >= 0 ? normalized.Substring(split + 1) : normalized.Trim('\\');
            if (string.IsNullOrWhiteSpace(path)) path = Program.ManagedTaskRoot;
            if (string.IsNullOrWhiteSpace(taskName)) throw new ArgumentException("Task name is required");
            string script =
                "$ErrorActionPreference='Stop';" +
                "$path='" + PsSingle(path) + "';" +
                "$action=New-ScheduledTaskAction -Execute '" + PsSingle(execute) + "'" + (string.IsNullOrWhiteSpace(arguments) ? ";" : " -Argument '" + PsSingle(arguments) + "';") +
                "$trigger=New-ScheduledTaskTrigger -AtLogOn;" +
                "$principal=New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited;" +
                "$settings=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 0);" +
                "Register-ScheduledTask -TaskPath $path -TaskName '" + PsSingle(taskName) + "' -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null;";
            string encoded = Convert.ToBase64String(Encoding.Unicode.GetBytes(script));
            RunChecked("powershell.exe", "-NoProfile -ExecutionPolicy Bypass -EncodedCommand " + encoded);
        }

        private static string PsSingle(string value) { return (value ?? "").Replace("'", "''"); }

        private static bool IsSelfTarget(string targetPath)
        {
            try
            {
                string self = Path.GetFullPath(Process.GetCurrentProcess().MainModule.FileName).TrimEnd('\\');
                string target = Path.GetFullPath(targetPath ?? "").TrimEnd('\\');
                return string.Equals(self, target, StringComparison.OrdinalIgnoreCase);
            }
            catch { return false; }
        }

        public static void DeleteManagedTask(string nameOrTask)
        {
            string tn = nameOrTask.StartsWith("\\") ? nameOrTask : Program.ManagedTaskRoot + nameOrTask.Replace(Program.ManagedTaskRoot.Trim('\\'), "").Trim('\\');
            RunChecked("schtasks.exe", "/Delete /F /TN " + Q(tn));
        }

        public static bool CommandUsesTrayWrapper(string command)
        {
            return Regex.IsMatch(command ?? "", @"--tray-run\s+[A-Za-z0-9+/=]+|--start-in-tray\b", RegexOptions.IgnoreCase);
        }

        public static void TogglePopupMode(StartupItem item)
        {
            SetPopupMode(item, !item.PopupEnabled());
        }

        public static void SetPopupMode(StartupItem item, bool popupEnabled)
        {
            string target, arguments;
            ResolveLaunchTarget(item, out target, out arguments);
            string execute = popupEnabled ? target : Process.GetCurrentProcess().MainModule.FileName;
            string actionArgs = popupEnabled ? (IsSelfTarget(target) ? "" : (arguments ?? "")) : (IsSelfTarget(target) ? "--start-in-tray" : "--tray-run " + Convert.ToBase64String(Encoding.UTF8.GetBytes(target + "\n" + (arguments ?? ""))));

            if (item.Id.StartsWith("task|") && item.IsManaged)
            {
                RegisterLogonTaskAt(item.Location, execute, actionArgs);
                return;
            }

            if (item.Enabled && item.CanDisable) Disable(item);
            else if (item.Enabled && !item.CanDisable) throw new InvalidOperationException("This startup source is read-only here. Select its matching Registry Run, Startup Folder, or Scheduled Task row if Windows exposes one.");

            AddManagedStartup(item.Name, target, arguments, !popupEnabled, true);
        }

        public static void ResolveLaunchTarget(StartupItem item, out string targetPath, out string arguments)
        {
            targetPath = ""; arguments = "";
            if (item == null) throw new ArgumentException("No startup item selected");
            string command = item.Command ?? "";
            if (item.Source == "Startup Folder" && command.EndsWith(".lnk", StringComparison.OrdinalIgnoreCase))
            {
                ResolveShortcut(command, out targetPath, out arguments);
                if (File.Exists(targetPath)) return;
            }
            if (TryDecodeTrayPayload(command, out targetPath, out arguments) && File.Exists(targetPath)) return;
            if (!TrySplitCommand(command, out targetPath, out arguments)) throw new InvalidOperationException("Could not parse executable from command");
            if (targetPath.EndsWith(".lnk", StringComparison.OrdinalIgnoreCase)) ResolveShortcut(targetPath, out targetPath, out arguments);
            if (!File.Exists(targetPath) || !targetPath.EndsWith(".exe", StringComparison.OrdinalIgnoreCase)) throw new FileNotFoundException("Resolved startup target is not a local .exe", targetPath);
        }

        private static bool TryDecodeTrayPayload(string command, out string targetPath, out string arguments)
        {
            targetPath = ""; arguments = "";
            Match m = Regex.Match(command ?? "", @"--tray-run\s+(?<payload>[A-Za-z0-9+/=]+)", RegexOptions.IgnoreCase);
            if (!m.Success) return false;
            try
            {
                string decoded = Encoding.UTF8.GetString(Convert.FromBase64String(m.Groups["payload"].Value));
                string[] lines = decoded.Split(new[] { '\n' }, 2);
                targetPath = lines[0]; arguments = lines.Length > 1 ? lines[1] : "";
                return true;
            }
            catch { return false; }
        }

        private static bool TrySplitCommand(string command, out string exe, out string args)
        {
            exe = ""; args = "";
            command = (command ?? "").Trim();
            if (command.Length == 0) return false;
            if (command.StartsWith("\"", StringComparison.Ordinal))
            {
                int end = command.IndexOf('"', 1);
                if (end > 1) { exe = command.Substring(1, end - 1); args = command.Substring(end + 1).Trim(); return true; }
            }
            Match m = Regex.Match(command, @"^(?<exe>.+?\.exe|.+?\.lnk)(?:\s+(?<args>.*))?$", RegexOptions.IgnoreCase);
            if (m.Success) { exe = m.Groups["exe"].Value.Trim(); args = m.Groups["args"].Value.Trim(); return true; }
            if (File.Exists(command)) { exe = command; args = ""; return true; }
            return false;
        }

        private static void ResolveShortcut(string shortcutPath, out string targetPath, out string arguments)
        {
            targetPath = shortcutPath; arguments = "";
            try
            {
                Type shellType = Type.GetTypeFromProgID("WScript.Shell");
                if (shellType == null) return;
                dynamic shell = Activator.CreateInstance(shellType);
                dynamic shortcut = shell.CreateShortcut(shortcutPath);
                targetPath = Convert.ToString(shortcut.TargetPath ?? shortcutPath);
                arguments = Convert.ToString(shortcut.Arguments ?? "");
            }
            catch { }
        }

        public static string ToJson(List<StartupItem> items)
        {
            var sb = new StringBuilder(); sb.Append("[");
            for (int i = 0; i < items.Count; i++)
            {
                if (i > 0) sb.Append(','); var x = items[i];
                sb.Append("{\"name\":\"").Append(Esc(x.Name)).Append("\",\"source\":\"").Append(Esc(x.Source)).Append("\",\"enabled\":").Append(x.Enabled ? "true" : "false").Append(",\"command\":\"").Append(Esc(x.Command)).Append("\",\"popup\":\"").Append(x.PopupLabel()).Append("\"}");
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

    internal static class ProtectedDisabledService
    {
        private static string Key(StartupItem item)
        {
            if (item == null) return "";
            if (item.Id.StartsWith("disabled|reg|")) return "reg|" + item.Scope + "|" + (item.Status ?? "").Split('\t')[0];
            if (item.Id.StartsWith("reg|")) return item.Id;
            if (item.Id.StartsWith("disabled|folder|")) return "folder|" + item.Scope + "|" + item.Status;
            if (item.Id.StartsWith("folder|")) return item.Id;
            if (item.Id.StartsWith("task|")) return "task|" + item.Location;
            return item.Id;
        }

        public static void Protect(StartupItem item)
        {
            if (item == null) return;
            string key = Key(item);
            if (string.IsNullOrWhiteSpace(key)) return;
            Directory.CreateDirectory(Program.AppData);
            var rows = LoadRows().Where(r => !string.Equals(r.Key, key, StringComparison.OrdinalIgnoreCase)).ToList();
            rows.Add(Row.FromItem(key, item));
            SaveRows(rows);
        }

        public static void Unprotect(StartupItem item)
        {
            string key = Key(item);
            if (string.IsNullOrWhiteSpace(key) || !File.Exists(Program.ProtectedDisabledStore)) return;
            SaveRows(LoadRows().Where(r => !string.Equals(r.Key, key, StringComparison.OrdinalIgnoreCase)).ToList());
        }

        public static string ProtectCurrentDisabled()
        {
            int count = 0;
            foreach (var item in StartupService.ScanAll().Where(x => !x.Enabled && !x.Id.StartsWith("error|"))) { Protect(item); count++; }
            return "PROTECT_DISABLED count=" + count + " store=" + Program.ProtectedDisabledStore;
        }

        public static string EnforceProtected()
        {
            int disabled = 0, failures = 0;
            foreach (var r in LoadRows())
            {
                try { if (EnforceRow(r)) disabled++; }
                catch { failures++; }
            }
            return "ENFORCE_DISABLED protected=" + LoadRows().Count + " actions=" + disabled + " failures=" + failures;
        }

        private static bool EnforceRow(Row r)
        {
            if (r.Key.StartsWith("task|", StringComparison.OrdinalIgnoreCase))
            {
                RunHidden("schtasks.exe", "/Change /TN " + Q(r.Location) + " /Disable");
                return true;
            }
            if (r.Key.StartsWith("reg|", StringComparison.OrdinalIgnoreCase) || r.Type == "Registry Run")
            {
                string valueName = (r.Status ?? "").Split('\t')[0];
                if (string.IsNullOrWhiteSpace(valueName)) valueName = r.Name;
                RegistryKey root = (r.Scope ?? "").Equals("Machine", StringComparison.OrdinalIgnoreCase) ? Registry.LocalMachine : Registry.CurrentUser;
                using (var key = root.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run", true))
                {
                    if (key != null && key.GetValueNames().Any(n => string.Equals(n, valueName, StringComparison.OrdinalIgnoreCase))) { key.DeleteValue(valueName, false); return true; }
                }
                return false;
            }
            if (r.Key.StartsWith("folder|", StringComparison.OrdinalIgnoreCase) || r.Type == "Startup Folder")
            {
                string original = !string.IsNullOrWhiteSpace(r.Status) ? r.Status : r.Command;
                if (!string.IsNullOrWhiteSpace(original) && File.Exists(original))
                {
                    Directory.CreateDirectory(Program.DisabledStartupFolder);
                    string dest = Path.Combine(Program.DisabledStartupFolder, Path.GetFileName(original) + ".protected." + DateTime.Now.Ticks + ".disabled");
                    File.Move(original, dest);
                    return true;
                }
            }
            return false;
        }

        private static List<Row> LoadRows()
        {
            var list = new List<Row>();
            if (!File.Exists(Program.ProtectedDisabledStore)) return list;
            foreach (var line in File.ReadAllLines(Program.ProtectedDisabledStore))
            {
                var p = line.Split('\t'); if (p.Length < 8) continue;
                list.Add(new Row { Key = UnB64(p[0]), Id = UnB64(p[1]), Type = UnB64(p[2]), Name = UnB64(p[3]), Scope = UnB64(p[4]), Command = UnB64(p[5]), Location = UnB64(p[6]), Status = UnB64(p[7]) });
            }
            return list;
        }

        private static void SaveRows(List<Row> rows)
        {
            Directory.CreateDirectory(Program.AppData);
            File.WriteAllLines(Program.ProtectedDisabledStore, rows.Select(r => string.Join("\t", new[] { B64(r.Key), B64(r.Id), B64(r.Type), B64(r.Name), B64(r.Scope), B64(r.Command), B64(r.Location), B64(r.Status) })), Encoding.UTF8);
        }

        private static void RunHidden(string exe, string args)
        {
            var psi = new ProcessStartInfo(exe, args) { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true };
            using (var p = Process.Start(psi)) { p.WaitForExit(15000); }
        }
        private static string Q(string s) { return "\"" + (s ?? "").Replace("\"", "\\\"") + "\""; }
        private static string B64(string s) { return Convert.ToBase64String(Encoding.UTF8.GetBytes(s ?? "")); }
        private static string UnB64(string s) { try { return Encoding.UTF8.GetString(Convert.FromBase64String(s)); } catch { return ""; } }
        private sealed class Row
        {
            public string Key, Id, Type, Name, Scope, Command, Location, Status;
            public static Row FromItem(string key, StartupItem item) { return new Row { Key = key, Id = item.Id, Type = item.Source, Name = item.Name, Scope = item.Scope, Command = item.Command, Location = item.Location, Status = item.Status }; }
        }
    }

    internal static class TrayRunner
    {
        private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
        [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] private static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
        private const int SW_HIDE = 0;
        private const int SW_MINIMIZE = 6;

        public static void Run(string[] args)
        {
            if (args.Length < 1) return;
            string decoded;
            try { decoded = Encoding.UTF8.GetString(Convert.FromBase64String(args[0])); }
            catch { return; }
            string[] lines = decoded.Split(new[] { '\n' }, 2);
            string target = lines[0];
            string targetArgs = lines.Length > 1 ? lines[1] : "";
            try
            {
                string full = Path.GetFullPath(target);
                if (!File.Exists(full) || !full.EndsWith(".exe", StringComparison.OrdinalIgnoreCase)) return;
                var psi = new ProcessStartInfo(full, targetArgs)
                {
                    UseShellExecute = false,
                    CreateNoWindow = true,
                    WindowStyle = ProcessWindowStyle.Hidden,
                    WorkingDirectory = Path.GetDirectoryName(full)
                };
                Process child = Process.Start(psi);
                if (child != null) HideProcessWindows(child, TimeSpan.FromSeconds(30));
            }
            catch { }
        }

        private static void HideProcessWindows(Process child, TimeSpan duration)
        {
            DateTime until = DateTime.UtcNow.Add(duration);
            int pid = child.Id;
            while (DateTime.UtcNow < until)
            {
                try { if (child.HasExited) return; child.Refresh(); } catch { return; }
                HideWindowsForPid(pid);
                System.Threading.Thread.Sleep(250);
            }
            HideWindowsForPid(pid);
        }

        private static void HideWindowsForPid(int pid)
        {
            EnumWindows((hWnd, lParam) =>
            {
                uint owner;
                GetWindowThreadProcessId(hWnd, out owner);
                if (owner == pid && IsWindowVisible(hWnd))
                {
                    ShowWindowAsync(hWnd, SW_HIDE);
                    ShowWindowAsync(hWnd, SW_MINIMIZE);
                    ShowWindowAsync(hWnd, SW_HIDE);
                }
                return true;
            }, IntPtr.Zero);
        }
    }

    internal sealed class MainForm : Form
    {
        private List<StartupItem> _items = new List<StartupItem>();
        private ListView _list;
        private TextBox _search;
        private Label _summary, _visibleValue, _enabledValue, _disabledValue, _reviewValue, _managedValue, _hint;
        private Button _refresh, _disable, _enable, _add, _deleteManaged, _clearSearch;
        private NotifyIcon _tray;
        private Timer _guardTimer;
        private bool _reallyExit;
        private readonly bool _startInTray;
        private readonly Color Bg = Color.FromArgb(8, 12, 26), Surface = Color.FromArgb(17, 24, 44), Surface2 = Color.FromArgb(24, 33, 58), Accent = Color.FromArgb(99, 102, 241), TextMain = Color.FromArgb(245, 247, 255), Muted = Color.FromArgb(156, 166, 195), Good = Color.FromArgb(52, 211, 153), Danger = Color.FromArgb(248, 113, 113), Warn = Color.FromArgb(251, 191, 36);

        public static string UiContractJson()
        {
            return "{\"columns\":[\"Status\",\"App / item\",\"Source\",\"Trust\",\"Location\",\"Popup\",\"Launch command\"],\"popupEnabledLabel\":\"Enabled\",\"popupDisabledLabel\":\"Disabled\",\"oneClickPopupToggle\":true,\"trayIcon\":true,\"trayDoubleClickOpens\":true,\"startInTrayArgument\":\"--start-in-tray\"}";
        }

        public MainForm(bool startInTray = false)
        {
            _startInTray = startInTray;
            Text = "Mich Startup Master — Windows Boot Control";
            Width = 1320; Height = 860; MinimumSize = new Size(1060, 720);
            BackColor = Bg; Font = new Font("Segoe UI", 10f); DoubleBuffered = true; Icon = Program.AppIcon;
            BuildUi(); BuildTray();
            Load += (s, e) => { ProtectedDisabledService.EnforceProtected(); RefreshItems(); ProtectedDisabledService.ProtectCurrentDisabled(); };
            _guardTimer = new Timer { Interval = 30000 };
            _guardTimer.Tick += (s, e) => ProtectedDisabledService.EnforceProtected();
            _guardTimer.Start();
            FormClosing += OnClosingToTray;
            Resize += (s, e) => { if (WindowState == FormWindowState.Minimized) HideToTray(); };
            Shown += (s, e) => { if (_startInTray) BeginInvoke(new Action(HideToTray)); };
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            using (var b = new LinearGradientBrush(ClientRectangle, Color.FromArgb(6, 10, 24), Color.FromArgb(36, 23, 72), 35f)) e.Graphics.FillRectangle(b, ClientRectangle);
            using (var glow = new SolidBrush(Color.FromArgb(45, 99, 102, 241))) e.Graphics.FillEllipse(glow, Width - 360, -160, 520, 360);
            base.OnPaint(e);
        }

        private void BuildUi()
        {
            var hero = Card(new Rectangle(28, 24, Width - 72, 144));
            hero.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right;
            Controls.Add(hero);
            var title = new Label { Text = "Startup Master", ForeColor = TextMain, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 28f), AutoSize = true, Location = new Point(24, 18) };
            var sub = new Label { Text = "A beautiful control room for everything that runs when Windows starts — registry, folders, scheduled tasks, normal launch, or quiet tray launch.", ForeColor = Muted, BackColor = Color.Transparent, Font = new Font("Segoe UI", 11.5f), AutoSize = false, Width = 820, Height = 44, Location = new Point(28, 68) };
            _summary = new Label { ForeColor = Color.White, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 10.5f), AutoSize = true, Location = new Point(280, 112) };
            hero.Controls.Add(title); hero.Controls.Add(sub); hero.Controls.Add(_summary);
            _add = Button("＋ Add app: Normal or Tray", Accent, 230); _add.Location = new Point(28, 100); _add.Click += (s, e) => AddBootApp(); hero.Controls.Add(_add);
            _refresh = Button("Refresh", Color.FromArgb(59, 130, 246), 120); _refresh.Location = new Point(hero.Width - 170, 92); _refresh.Anchor = AnchorStyles.Top | AnchorStyles.Right; _refresh.Click += (s, e) => RefreshItems(); hero.Controls.Add(_refresh);

            int cardTop = 188, cardW = 184, gap = 14;
            _visibleValue = MetricCard("Visible", "startup items in view", Color.FromArgb(129, 140, 248), 28 + (cardW + gap) * 0, cardTop, cardW);
            _enabledValue = MetricCard("Enabled", "will run at boot", Good, 28 + (cardW + gap) * 1, cardTop, cardW);
            _disabledValue = MetricCard("Disabled", "kept from startup", Danger, 28 + (cardW + gap) * 2, cardTop, cardW);
            _reviewValue = MetricCard("Review", "commands to inspect", Warn, 28 + (cardW + gap) * 3, cardTop, cardW);
            _managedValue = MetricCard("Managed", "created here", Accent, 28 + (cardW + gap) * 4, cardTop, cardW);

            var toolbar = Card(new Rectangle(28, 292, Width - 72, 70));
            toolbar.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right; Controls.Add(toolbar);
            var searchLabel = new Label { Text = "Search", ForeColor = Muted, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 9f), Location = new Point(18, 8), AutoSize = true }; toolbar.Controls.Add(searchLabel);
            _search = StyledTextBox(); _search.Location = new Point(18, 30); _search.Width = 410; _search.TextChanged += (s, e) => RenderList(); toolbar.Controls.Add(_search);
            _clearSearch = Button("Clear", Surface2, 82); _clearSearch.Location = new Point(440, 26); _clearSearch.Height = 32; _clearSearch.Click += (s, e) => { _search.Text = ""; }; toolbar.Controls.Add(_clearSearch);
            _disable = Button("Disable selected", Danger, 150); _disable.Location = new Point(toolbar.Width - 470, 18); _disable.Anchor = AnchorStyles.Top | AnchorStyles.Right; _disable.Click += (s, e) => DisableSelected(); toolbar.Controls.Add(_disable);
            _enable = Button("Enable selected", Good, 145); _enable.Location = new Point(toolbar.Width - 310, 18); _enable.Anchor = AnchorStyles.Top | AnchorStyles.Right; _enable.Click += (s, e) => EnableSelected(); toolbar.Controls.Add(_enable);
            _deleteManaged = Button("Delete managed", Color.FromArgb(234, 179, 8), 145); _deleteManaged.Location = new Point(toolbar.Width - 155, 18); _deleteManaged.Anchor = AnchorStyles.Top | AnchorStyles.Right; _deleteManaged.Click += (s, e) => DeleteManaged(); toolbar.Controls.Add(_deleteManaged);

            var listCard = Card(new Rectangle(28, 382, Width - 72, Height - 438));
            listCard.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right | AnchorStyles.Bottom; Controls.Add(listCard);
            _hint = new Label { Text = "Popup shows whether startup can pop a window: Enabled = normal launch, Disabled = silent tray wrapper. Click the Popup value once to switch.", ForeColor = Muted, BackColor = Color.Transparent, Font = new Font("Segoe UI", 9.5f), Location = new Point(18, 12), AutoSize = true }; listCard.Controls.Add(_hint);
            _list = new ListView { Location = new Point(18, 42), Size = new Size(listCard.Width - 36, listCard.Height - 60), Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right | AnchorStyles.Bottom, View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.None, BackColor = Color.FromArgb(12, 18, 34), ForeColor = TextMain, Font = new Font("Segoe UI", 9.7f), HideSelection = false, OwnerDraw = true };
            _list.SmallImageList = new ImageList { ImageSize = new Size(1, 34) };
            _list.Columns.Add("Status", 105); _list.Columns.Add("App / item", 230); _list.Columns.Add("Source", 135); _list.Columns.Add("Trust", 85); _list.Columns.Add("Popup", 125); _list.Columns.Add("Location", 240); _list.Columns.Add("Launch command", 430);
            _list.DrawColumnHeader += (s, e) => { using (var b = new SolidBrush(Surface2)) e.Graphics.FillRectangle(b, e.Bounds); TextRenderer.DrawText(e.Graphics, e.Header.Text, new Font(Font, FontStyle.Bold), new Rectangle(e.Bounds.X + 8, e.Bounds.Y, e.Bounds.Width, e.Bounds.Height), Color.White, TextFormatFlags.VerticalCenter | TextFormatFlags.Left); };
            _list.DrawSubItem += DrawSubItem; _list.SelectedIndexChanged += (s, e) => UpdateButtons(); _list.MouseUp += ListMouseUpPopupToggle;
            _list.Resize += (s, e) => { if (_list.Columns.Count > 6) _list.Columns[6].Width = Math.Max(300, _list.Width - 1030); };
            listCard.Controls.Add(_list);
        }

        private Panel Card(Rectangle bounds)
        {
            return new Panel { Bounds = bounds, BackColor = Color.FromArgb(220, Surface), BorderStyle = BorderStyle.FixedSingle };
        }

        private Label MetricCard(string title, string helper, Color accent, int x, int y, int w)
        {
            var card = Card(new Rectangle(x, y, w, 84)); card.Anchor = AnchorStyles.Top | AnchorStyles.Left; Controls.Add(card);
            var stripe = new Panel { BackColor = accent, Location = new Point(0, 0), Size = new Size(5, 84) }; card.Controls.Add(stripe);
            var value = new Label { Text = "0", ForeColor = TextMain, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 22f), Location = new Point(18, 8), AutoSize = true }; card.Controls.Add(value);
            card.Controls.Add(new Label { Text = title, ForeColor = accent, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 9.5f), Location = new Point(20, 50), AutoSize = true });
            card.Controls.Add(new Label { Text = helper, ForeColor = Muted, BackColor = Color.Transparent, Font = new Font("Segoe UI", 8.5f), Location = new Point(20, 66), AutoSize = true });
            return value;
        }

        private void DrawSubItem(object sender, DrawListViewSubItemEventArgs e)
        {
            var item = (StartupItem)e.Item.Tag; bool selected = e.Item.Selected;
            Color row = selected ? Color.FromArgb(52, 64, 116) : (e.ItemIndex % 2 == 0 ? Color.FromArgb(13, 20, 38) : Color.FromArgb(16, 24, 45));
            using (var b = new SolidBrush(row)) e.Graphics.FillRectangle(b, e.Bounds);
            if (e.ColumnIndex == 4) { DrawPopupToggle(e.Graphics, e.Bounds, item); return; }
            Color c = TextMain; string text = e.SubItem.Text;
            if (e.ColumnIndex == 0) { c = item.Enabled ? Good : Danger; text = item.Enabled ? "● Enabled" : "● Disabled"; }
            if (e.ColumnIndex == 3) c = item.RiskLabel() == "Review" ? Warn : Good;
            if (e.ColumnIndex == 5 || e.ColumnIndex == 6) c = Muted;
            TextRenderer.DrawText(e.Graphics, text, _list.Font, new Rectangle(e.Bounds.X + 10, e.Bounds.Y, e.Bounds.Width - 12, e.Bounds.Height), c, TextFormatFlags.VerticalCenter | TextFormatFlags.EndEllipsis | TextFormatFlags.Left);
        }

        private void DrawPopupToggle(Graphics g, Rectangle bounds, StartupItem item)
        {
            bool popupEnabled = item.PopupEnabled();
            Rectangle r = PopupButtonRect(bounds);
            Color color = popupEnabled ? Warn : Good;
            string text = popupEnabled ? "Enabled" : "Disabled";
            using (var b = new SolidBrush(color)) g.FillRectangle(b, r);
            using (var p = new Pen(Color.FromArgb(190, Color.White))) g.DrawRectangle(p, r);
            TextRenderer.DrawText(g, text, new Font(_list.Font, FontStyle.Bold), r, Color.FromArgb(10, 14, 28), TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter | TextFormatFlags.EndEllipsis);
        }

        private Rectangle PopupButtonRect(Rectangle bounds)
        {
            int w = Math.Max(92, bounds.Width - 18);
            int h = Math.Min(26, bounds.Height - 8);
            int y = bounds.Y + (bounds.Height - h) / 2;
            int x = bounds.X + Math.Max(6, (bounds.Width - w) / 2);
            return new Rectangle(x, y, w, h);
        }

        private TextBox StyledTextBox() { return new TextBox { BorderStyle = BorderStyle.FixedSingle, BackColor = Color.FromArgb(10, 16, 31), ForeColor = Color.White, Font = new Font("Segoe UI", 11f), Height = 30 }; }
        private Button Button(string text, Color color, int width)
        {
            var b = new Button { Text = text, Width = width, Height = 40, FlatStyle = FlatStyle.Flat, BackColor = color, ForeColor = Color.White, Font = new Font("Segoe UI Semibold", 9.5f), Cursor = Cursors.Hand };
            b.FlatAppearance.BorderSize = 0; b.MouseEnter += (s, e) => b.BackColor = ControlPaint.Light(color, .10f); b.MouseLeave += (s, e) => b.BackColor = color; return b;
        }

        private void BuildTray()
        {
            _tray = new NotifyIcon { Icon = Program.AppIcon, Text = "Mich Startup Master", Visible = true };
            _tray.DoubleClick += (s, e) => OpenFromTray();
            _tray.MouseDoubleClick += (s, e) => { if (e.Button == MouseButtons.Left) OpenFromTray(); };
            _tray.ContextMenu = new ContextMenu(new[] { new MenuItem("Open Startup Master", (s, e) => OpenFromTray()), new MenuItem("Refresh inventory", (s, e) => RefreshItems()), new MenuItem("Exit", (s, e) => { _reallyExit = true; if (_tray != null) { _tray.Visible = false; _tray.Dispose(); } Application.Exit(); }) });
        }
        private void OpenFromTray()
        {
            if (IsDisposed) return;
            if (InvokeRequired) { BeginInvoke(new Action(OpenFromTray)); return; }
            ShowInTaskbar = true;
            Show();
            if (WindowState == FormWindowState.Minimized) WindowState = FormWindowState.Normal;
            BringToFront();
            Activate();
            TopMost = true;
            TopMost = false;
        }
        private void HideToTray()
        {
            if (_tray != null) _tray.Visible = true;
            ShowInTaskbar = false;
            Hide();
        }
        private void OnClosingToTray(object sender, FormClosingEventArgs e) { if (!_reallyExit && _tray != null && _tray.Visible && e.CloseReason == CloseReason.UserClosing) { e.Cancel = true; HideToTray(); } }
        private void RefreshItems() { Cursor = Cursors.WaitCursor; try { _items = StartupService.ScanAll(); RenderList(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Refresh failed", MessageBoxButtons.OK, MessageBoxIcon.Error); } finally { Cursor = Cursors.Default; } }
        private void RenderList()
        {
            string q = (_search.Text ?? "").Trim().ToLowerInvariant();
            var rows = _items.Where(x => string.IsNullOrEmpty(q) || (x.Name + " " + x.Command + " " + x.Source + " " + x.Location).ToLowerInvariant().Contains(q)).ToList();
            _list.BeginUpdate(); _list.Items.Clear(); foreach (var x in rows) { var li = new ListViewItem(x.Enabled ? "Enabled" : "Disabled") { Tag = x }; li.SubItems.Add(x.Name); li.SubItems.Add(x.Source); li.SubItems.Add(x.RiskLabel()); li.SubItems.Add(x.PopupLabel()); li.SubItems.Add(x.Location); li.SubItems.Add(x.Command); _list.Items.Add(li); } _list.EndUpdate();
            int review = rows.Count(x => x.RiskLabel() == "Review");
            _summary.Text = rows.Count + " visible • " + _items.Count(x => x.Enabled) + " enabled • " + _items.Count(x => !x.Enabled) + " disabled • " + _items.Count(x => x.IsManaged) + " managed";
            _visibleValue.Text = rows.Count.ToString(); _enabledValue.Text = _items.Count(x => x.Enabled).ToString(); _disabledValue.Text = _items.Count(x => !x.Enabled).ToString(); _reviewValue.Text = review.ToString(); _managedValue.Text = _items.Count(x => x.IsManaged).ToString();
            _hint.Text = rows.Count == 0 ? "No startup items match this search. Clear the search to return to the full boot inventory." : "Popup: Enabled means normal startup may show a window; Disabled means Startup Master's tray wrapper starts it quietly. Click a Popup cell once to switch.";
            UpdateButtons();
        }
        private void ListMouseUpPopupToggle(object sender, MouseEventArgs e)
        {
            var hit = _list.HitTest(e.Location);
            if (hit.Item == null || hit.SubItem == null) return;
            int col = hit.Item.SubItems.IndexOf(hit.SubItem);
            if (col != 4) return;
            if (!PopupButtonRect(hit.SubItem.Bounds).Contains(e.Location)) return;
            ToggleItemPopupState((StartupItem)hit.Item.Tag);
        }

        private void ToggleItemPopupState(StartupItem item)
        {
            try
            {
                bool wasEnabled = item.PopupEnabled();
                StartupService.TogglePopupMode(item);
                _hint.Text = item.Name + " popup is now " + (wasEnabled ? "Disabled — it will start through the silent tray wrapper." : "Enabled — it will start normally and may show a window.");
                RefreshItems();
            }
            catch (Exception ex)
            {
                _hint.Text = "Could not change Popup for " + item.Name + ": " + ex.Message;
            }
        }

        private void UpdateButtons() { var x = Selected(); bool any = x != null; _disable.Enabled = any && x.Enabled; _enable.Enabled = any && !x.Enabled; _deleteManaged.Enabled = any && x.IsManaged && x.Source == "Scheduled Task"; }
        private StartupItem Selected() { return _list.SelectedItems.Count == 0 ? null : (StartupItem)_list.SelectedItems[0].Tag; }
        private void DisableSelected() { var x = Selected(); if (x == null) return; if (MessageBox.Show("Disable '" + x.Name + "' from Windows startup?", "Confirm disable", MessageBoxButtons.YesNo, MessageBoxIcon.Question) != DialogResult.Yes) return; try { StartupService.Disable(x); Toast("Disabled", x.Name + " will not run next boot."); RefreshItems(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Disable failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); } }
        private void EnableSelected() { var x = Selected(); if (x == null) return; try { StartupService.Enable(x); Toast("Enabled", x.Name + " will run next boot."); RefreshItems(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Enable failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); } }
        private void DeleteManaged() { var x = Selected(); if (x == null || !x.IsManaged || x.Source != "Scheduled Task") { MessageBox.Show("Select a MichStartupMaster managed scheduled task."); return; } if (MessageBox.Show("Delete managed startup task '" + x.Name + "'?", "Confirm delete", MessageBoxButtons.YesNo, MessageBoxIcon.Warning) != DialogResult.Yes) return; try { StartupService.DeleteManagedTask(x.Location); RefreshItems(); Toast("Deleted", x.Name); } catch (Exception ex) { MessageBox.Show(ex.Message, "Delete failed"); } }
        private void AddBootApp()
        {
            using (var d = new AddStartupForm())
            {
                if (d.ShowDialog(this) != DialogResult.OK) return;
                try { StartupService.AddManagedStartup(d.AppTitle, d.AppPath, d.AppArguments, d.TrayMode, true); Toast("Added zero-delay boot app", d.AppTitle + (d.TrayMode ? " will start quietly through tray mode." : " will start normally.")); RefreshItems(); }
                catch (Exception ex) { MessageBox.Show(ex.Message, "Add failed", MessageBoxButtons.OK, MessageBoxIcon.Error); }
            }
        }
        private void Toast(string title, string body) { _hint.Text = title + ": " + body; }
    }

    internal sealed class AddStartupForm : Form
    {
        public string AppTitle { get { return _name.Text.Trim(); } }
        public string AppPath { get { return _path.Text.Trim(); } }
        public string AppArguments { get { return _args.Text; } }
        public bool TrayMode { get { return _trayMode.Checked; } }
        private TextBox _name, _path, _args;
        private RadioButton _normalMode, _trayMode;
        private readonly Color Bg = Color.FromArgb(10, 14, 28), Surface = Color.FromArgb(21, 28, 51), TextMain = Color.FromArgb(245, 247, 255), Muted = Color.FromArgb(156, 166, 195), Accent = Color.FromArgb(99, 102, 241);

        public AddStartupForm()
        {
            Text = "Add app to Windows startup"; Width = 720; Height = 520; FormBorderStyle = FormBorderStyle.FixedDialog; MaximizeBox = false; MinimizeBox = false; BackColor = Bg; ForeColor = Color.White; Font = new Font("Segoe UI", 10f); StartPosition = FormStartPosition.CenterParent; Icon = Program.AppIcon;
            Controls.Add(new Label { Text = "Add an app to startup", Left = 28, Top = 24, AutoSize = true, ForeColor = TextMain, Font = new Font("Segoe UI Semibold", 22f) });
            Controls.Add(new Label { Text = "Choose an executable, then decide if it should open normally or quietly through Startup Master's tray wrapper.", Left = 30, Top = 66, Width = 620, Height = 40, ForeColor = Muted, Font = new Font("Segoe UI", 10.5f) });
            AddLabel("Friendly name", 118); _name = Box(144, 470);
            AddLabel("Executable path", 184); _path = Box(210, 510); var browse = Button("Browse", Accent, 92); browse.Left = 558; browse.Top = 208; browse.Click += Browse; Controls.Add(browse);
            AddLabel("Optional arguments", 250); _args = Box(276, 622);
            AddLabel("Startup mode", 318);
            _normalMode = new RadioButton { Text = "Start normally — run the app directly at Windows logon", Left = 34, Top = 346, Width = 610, ForeColor = TextMain, BackColor = Bg, Checked = false };
            _trayMode = new RadioButton { Text = "Start quietly in tray mode — no terminal, minimized launch, controller tray icon", Left = 34, Top = 378, Width = 630, ForeColor = TextMain, BackColor = Bg, Checked = true };
            Controls.Add(_normalMode); Controls.Add(_trayMode);
            Controls.Add(new Label { Text = "Quiet tray mode is the safest generic no-popup startup path; apps that force their own window may still show it.", Left = 54, Top = 406, Width = 590, Height = 34, ForeColor = Muted, Font = new Font("Segoe UI", 9f) });
            var ok = Button("Add at next boot", Accent, 150); ok.Left = 390; ok.Top = 452; ok.DialogResult = DialogResult.OK; ok.Click += ValidateBeforeClose;
            var cancel = Button("Cancel", Surface, 100); cancel.Left = 550; cancel.Top = 452; cancel.DialogResult = DialogResult.Cancel;
            Controls.Add(ok); Controls.Add(cancel); AcceptButton = ok; CancelButton = cancel;
        }
        private void ValidateBeforeClose(object sender, EventArgs e)
        {
            if (string.IsNullOrWhiteSpace(AppTitle) || string.IsNullOrWhiteSpace(AppPath) || !File.Exists(AppPath) || !AppPath.EndsWith(".exe", StringComparison.OrdinalIgnoreCase))
            {
                MessageBox.Show("Choose a valid .exe and friendly name before adding startup.", "Missing app", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                DialogResult = DialogResult.None;
            }
        }
        private void AddLabel(string text, int top) { Controls.Add(new Label { Text = text, Left = 30, Top = top, AutoSize = true, ForeColor = Muted, Font = new Font("Segoe UI Semibold", 9.5f) }); }
        private TextBox Box(int top, int width) { var t = new TextBox { Left = 30, Top = top, Width = width, Height = 30, BackColor = Color.FromArgb(17, 24, 44), ForeColor = Color.White, BorderStyle = BorderStyle.FixedSingle, Font = new Font("Segoe UI", 10.5f) }; Controls.Add(t); return t; }
        private Button Button(string text, Color color, int width) { var b = new Button { Text = text, Width = width, Height = 38, FlatStyle = FlatStyle.Flat, BackColor = color, ForeColor = Color.White, Font = new Font("Segoe UI Semibold", 9.5f), Cursor = Cursors.Hand }; b.FlatAppearance.BorderSize = 0; return b; }
        private void Browse(object sender, EventArgs e) { using (var ofd = new OpenFileDialog { Filter = "Applications (*.exe)|*.exe", Title = "Choose app to start with Windows" }) if (ofd.ShowDialog(this) == DialogResult.OK) { _path.Text = ofd.FileName; if (string.IsNullOrWhiteSpace(_name.Text)) _name.Text = Path.GetFileNameWithoutExtension(ofd.FileName); } }
    }

}
