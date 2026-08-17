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
using System.Threading.Tasks;
using System.Windows.Forms;

namespace MichStartupMaster
{
    internal static class Program
    {
        public static readonly string AppName = "MichStartupMaster";
        public static readonly string AppData = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), AppName);

        [DllImport("user32.dll", CharSet = CharSet.Unicode)]
        private static extern IntPtr FindWindow(string lpClassName, string lpWindowName);
        [DllImport("user32.dll")] private static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr hWnd);
        public static readonly string DisabledStore = Path.Combine(AppData, "disabled-items.tsv");
        public static readonly string ProtectedDisabledStore = Path.Combine(AppData, "protected-disabled-items.tsv");
        public static readonly string ProtectedQuietStore = Path.Combine(AppData, "protected-quiet-popup-items.tsv");
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
                    string bundledIcon = Path.Combine(AppContext.BaseDirectory, "MichStartupMaster.ico");
                    if (File.Exists(bundledIcon))
                    {
                        _appIcon = new Icon(bundledIcon);
                        return _appIcon;
                    }
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
            Directory.CreateDirectory(AppData);
            if (args.Length > 0)
            {
                string cmd = args[0].ToLowerInvariant();
                if (cmd == "--smoke") return Smoke();
                if (cmd == "--version") { Console.WriteLine("MichStartupMaster GitHub recovery build"); return 0; }
                if (cmd == "--list") { Console.WriteLine(StartupService.ToJson(StartupService.ScanAll())); return 0; }
                if (cmd == "--add-test-task") return CliAddTestTask(args, true);
                if (cmd == "--add-test-task-tray") return CliAddTestTask(args, true);
                if (cmd == "--add-test-task-normal") return CliAddTestTask(args, false);
                if (cmd == "--add-startup") return CliAddStartup(args);
                if (cmd == "--remove-task") return CliRemoveTask(args);
                if (cmd == "--ui-contract") { Console.WriteLine(MainForm.UiContractJson()); return 0; }
                if (cmd == "--protect-disabled") { Console.WriteLine(ProtectedDisabledService.ProtectCurrentDisabled()); return 0; }
                if (cmd == "--enforce-disabled") { Console.WriteLine(ProtectedDisabledService.EnforceProtected()); return 0; }
                if (cmd == "--enforce-quiet") { Console.WriteLine(ProtectedQuietService.EnforceProtected()); return 0; }
                if (cmd == "--toggle-popup") return CliTogglePopup(args);
                if (cmd == "--set-enabled") return CliSetEnabled(args);
                if (cmd == "--tray-run") { TrayRunner.Run(args.Skip(1).ToArray()); return 0; }
                if (cmd == "--start-in-tray" || cmd == "--agent")
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

        private static int CliAddStartup(string[] args)
        {
            if (args.Length < 3) { Console.WriteLine("usage: --add-startup <name> <path> [arguments] [normal|tray]"); return 2; }
            string name = args[1];
            string target = args[2];
            string targetArgs = "";
            string mode = "tray";
            if (args.Length > 3)
            {
                if (string.Equals(args[3], "normal", StringComparison.OrdinalIgnoreCase) || string.Equals(args[3], "tray", StringComparison.OrdinalIgnoreCase)) mode = args[3];
                else { targetArgs = args[3]; if (args.Length > 4) mode = args[4]; }
            }
            bool trayMode = !string.Equals(mode, "normal", StringComparison.OrdinalIgnoreCase);
            string task = StartupService.AddManagedStartup(name, target, targetArgs, trayMode, true);
            bool exists = StartupService.ScanAll().Any(x => x.Location.Equals(task, StringComparison.OrdinalIgnoreCase) && x.Source == "Scheduled Task" && x.Enabled);
            Console.WriteLine("ADD_STARTUP task=" + task + " mode=" + (trayMode ? "tray" : "normal") + " exists=" + exists);
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

        private static int CliSetEnabled(string[] args)
        {
            if (args.Length < 3) { Console.WriteLine("usage: --set-enabled <name|location|id> <true|false>"); return 2; }
            string key = args[1];
            bool enabled;
            if (!bool.TryParse(args[2], out enabled)) { Console.WriteLine("enabled must be true or false"); return 2; }
            var item = StartupService.ScanAll().FirstOrDefault(x => x.Id.Equals(key, StringComparison.OrdinalIgnoreCase) || x.Location.Equals(key, StringComparison.OrdinalIgnoreCase) || x.Name.Equals(key.TrimStart('\\'), StringComparison.OrdinalIgnoreCase));
            if (item == null) { Console.WriteLine("startup item not found: " + key); return 4; }
            if (enabled) StartupService.Enable(item); else StartupService.Disable(item);
            var updated = StartupService.ScanAll().FirstOrDefault(x => x.Id.Equals(item.Id, StringComparison.OrdinalIgnoreCase) || x.Location.Equals(item.Location, StringComparison.OrdinalIgnoreCase) || x.Name.Equals(item.Name, StringComparison.OrdinalIgnoreCase));
            Console.WriteLine("SET_ENABLED " + item.Name + " requested=" + enabled + " now=" + (updated == null ? "unknown" : updated.Enabled.ToString()));
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
        public string AppName;
        public bool Enabled;
        public bool CanDisable;
        public bool IsManaged;
        public string Status;

        public string HumanName()
        {
            if (!string.IsNullOrWhiteSpace(AppName)) return AppName;
            return string.IsNullOrWhiteSpace(Name) ? "(startup item)" : Name;
        }

        public string PopupLabel()
        {
            if (Source == "Windows Service" || Source == "System Driver" || Source == "Winlogon Autostart" || Source == "AppInit DLLs" || Source == "Active Setup") return "N/A";
            if (!Enabled) return "Disabled";
            return StartupService.CommandUsesTrayWrapper(Command) ? "Disabled" : "Enabled";
        }

        public bool PopupEnabled()
        {
            return PopupLabel() == "Enabled";
        }

        public string RiskLabel()
        {
            string risk = RiskLevel();
            if (risk == "Critical") return "HIGH RISK";
            if (risk == "Review") return "Review";
            if (risk == "System") return "System";
            return "Normal";
        }

        public string RiskLevel()
        {
            string c = (Command ?? "").ToLowerInvariant();
            string n = (Name ?? "").ToLowerInvariant();
            string s = (Source ?? "").ToLowerInvariant();
            string st = (Status ?? "").ToLowerInvariant();
            if (s == "system driver" || s == "winlogon autostart" || s == "appinit dlls") return "Critical";
            if (s == "windows service" && IsSecurityOrCoreService(n + " " + c + " " + st)) return "Critical";
            if (s == "scheduled task" && st.Contains("boot") && (Location ?? "").StartsWith(@"\Microsoft\Windows\", StringComparison.OrdinalIgnoreCase)) return "Critical";
            if (c.Contains("temp") || c.Contains("appdata\\local\\temp") || c.Contains("powershell") || c.Contains("cmd.exe") || c.Contains("wscript.exe") || c.Contains("cscript.exe")) return "Review";
            if (Scope == "Machine") return "System";
            return "Normal";
        }

        private static bool IsSecurityOrCoreService(string text)
        {
            string[] needles = { "defender", "windefend", "security", "firewall", "antivirus", "anti-malware", "antimalware", "malwarebytes", "mbam", "vpn", "credential", "crypt", "event log", "rpc", "plugplay", "winmgmt" };
            return needles.Any(x => text.IndexOf(x, StringComparison.OrdinalIgnoreCase) >= 0);
        }

        public string RiskReason()
        {
            string s = Source ?? "";
            if (RiskLevel() == "Critical")
            {
                if (s == "System Driver") return "Boot/system driver. Disabling can break hardware, security, or Windows startup.";
                if (s == "Winlogon Autostart") return "Windows logon component. Wrong changes can break sign-in.";
                if (s == "AppInit DLLs") return "DLL injection startup point. Changes can affect many desktop apps.";
                if (s == "Windows Service") return "Security or core Windows service. Disabling can reduce protection or break system behavior.";
                return "Microsoft boot/startup infrastructure task.";
            }
            if (RiskLevel() == "Review") return "Script or shell-based startup command. Inspect before trusting.";
            if (RiskLevel() == "System") return "Machine-wide startup item.";
            return "Normal user/app startup item.";
        }

        public string AdviceLevel()
        {
            if (!Enabled || RiskLevel() == "Critical") return "Keep";
            string haystack = ((Name ?? "") + " " + (AppName ?? "") + " " + (Command ?? "") + " " + (Location ?? "") + " " + (Status ?? "")).ToLowerInvariant();
            bool optionalUpdater = Regex.IsMatch(haystack, @"\b(updater|update task|update check|updatecurrentuser|updatecurrentmachine|--wake|/wake)\b", RegexOptions.IgnoreCase);
            bool trayHelper = Regex.IsMatch(haystack, @"\b(tray icon|tray_icon|trayicon|helper tray|notification icon)\b", RegexOptions.IgnoreCase);
            bool telemetry = Regex.IsMatch(haystack, @"\b(telemetry|crash reporter|crashreporter|usage reporter|feedback uploader)\b", RegexOptions.IgnoreCase);
            bool installerWatcher = Regex.IsMatch(haystack, @"\b(installer dialog watchdog|auto.?install guardian|visible downloads live sync|popup rescue)\b", RegexOptions.IgnoreCase);
            if (optionalUpdater || trayHelper || telemetry || installerWatcher) return "Cleanup";
            return "Keep";
        }

        public string AdviceLabel()
        {
            return AdviceLevel() == "Cleanup" ? "REMOVE?" : "Keep";
        }

        public string AdviceReason()
        {
            if (AdviceLevel() != "Cleanup") return "No strong optional-startup cleanup signal.";
            string haystack = ((Name ?? "") + " " + (AppName ?? "") + " " + (Command ?? "") + " " + (Location ?? "") + " " + (Status ?? "")).ToLowerInvariant();
            if (haystack.Contains("telemetry") || haystack.Contains("crash reporter") || haystack.Contains("crashreporter")) return "Optional telemetry or crash-reporting startup helper.";
            if (haystack.Contains("tray icon") || haystack.Contains("tray_icon") || haystack.Contains("trayicon")) return "Optional tray-icon helper; usually safe to start manually instead.";
            if (haystack.Contains("installer dialog watchdog") || haystack.Contains("auto-install guardian") || haystack.Contains("popup rescue")) return "Installer/watchdog helper, not normally needed at every boot.";
            return "Updater/checker startup pattern; usually not necessary at every boot.";
        }
    }

    internal static class StartupService
    {
        public static List<StartupItem> ScanAll()
        {
            var items = new List<StartupItem>();
            AddWmiStartupCommands(items);
            AddCommonRegistryStartup(items);
            AddStartupFolder(items, Environment.GetFolderPath(Environment.SpecialFolder.Startup), "User");
            AddStartupFolder(items, Environment.GetFolderPath(Environment.SpecialFolder.CommonStartup), "Machine");
            AddLogonTasks(items);
            AddAutoServices(items);
            AddAutoDrivers(items);
            AddStartupApproved(items);
            items.AddRange(DisabledStoreService.LoadDisabledItems());
            HydrateHumanNames(items);
            return Dedupe(items).OrderBy(x => x.Enabled ? 0 : 1).ThenBy(x => x.Source).ThenBy(x => x.Name).ToList();
        }

        private static void HydrateHumanNames(List<StartupItem> items)
        {
            foreach (var item in items)
            {
                if (string.IsNullOrWhiteSpace(item.AppName)) item.AppName = FriendlyNameFor(item);
            }
        }

        private static string FriendlyNameFor(StartupItem item)
        {
            if (item == null) return "";
            string target, args;
            if (TryFriendlyTarget(item, out target, out args))
            {
                string fromFile = FriendlyNameFromFile(target);
                if (!string.IsNullOrWhiteSpace(fromFile)) return fromFile;
                return CleanName(SafeFileStem(target));
            }
            string cleaned = CleanName(item.Name);
            if (!string.IsNullOrWhiteSpace(cleaned)) return cleaned;
            return CleanName(item.Location);
        }

        private static bool TryFriendlyTarget(StartupItem item, out string target, out string args)
        {
            target = ""; args = "";
            try
            {
                if (item.Source == "Startup Folder" && (item.Command ?? "").EndsWith(".lnk", StringComparison.OrdinalIgnoreCase))
                {
                    ResolveShortcut(item.Command, out target, out args);
                    if (!string.IsNullOrWhiteSpace(target)) return true;
                }
                if (TryDecodeTrayPayload(item.Command ?? "", out target, out args)) return true;
                if (TrySplitCommand(ExpandPathTokens(item.Command ?? ""), out target, out args)) return true;
            }
            catch { }
            return false;
        }

        private static string FriendlyNameFromFile(string path)
        {
            try
            {
                string expanded = ExpandPathTokens(path);
                if (!File.Exists(expanded)) return "";
                FileVersionInfo info = FileVersionInfo.GetVersionInfo(expanded);
                string[] candidates = { info.ProductName, info.FileDescription, info.InternalName, Path.GetFileNameWithoutExtension(expanded) };
                foreach (string candidate in candidates)
                {
                    string cleaned = CleanName(candidate);
                    if (!string.IsNullOrWhiteSpace(cleaned)) return cleaned;
                }
            }
            catch { }
            return "";
        }

        private static string SafeFileStem(string value)
        {
            try { return Path.GetFileNameWithoutExtension(value); }
            catch { return value; }
        }

        private static string ExpandPathTokens(string value)
        {
            if (string.IsNullOrWhiteSpace(value)) return "";
            return Environment.ExpandEnvironmentVariables(value.Trim('"'));
        }

        private static string CleanName(string value)
        {
            if (string.IsNullOrWhiteSpace(value)) return "";
            string s = value.Trim();
            if (s.StartsWith("@", StringComparison.Ordinal)) s = s.Substring(1);
            int comma = s.IndexOf(',');
            if (comma > 0 && (s.IndexOf("\\", StringComparison.Ordinal) >= 0 || s.EndsWith(".dll", StringComparison.OrdinalIgnoreCase) || s.EndsWith(".exe", StringComparison.OrdinalIgnoreCase))) s = Path.GetFileNameWithoutExtension(s.Substring(0, comma));
            s = Regex.Replace(s, @"\{[0-9A-Fa-f-]{20,}\}", " ");
            s = Regex.Replace(s, @"[_-]?[0-9a-fA-F]{8,}$", " ");
            s = s.Replace("_", " ").Replace("-", " ");
            s = Regex.Replace(s, @"\s+", " ").Trim();
            if (s.Length == 0) return "";
            return s;
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

        private static void AddCommonRegistryStartup(List<StartupItem> items)
        {
            string[] runKeys = new[]
            {
                @"Software\Microsoft\Windows\CurrentVersion\Run",
                @"Software\Microsoft\Windows\CurrentVersion\RunOnce",
                @"Software\Microsoft\Windows\CurrentVersion\RunServices",
                @"Software\Microsoft\Windows\CurrentVersion\RunServicesOnce",
                @"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer\Run",
                @"Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Run",
                @"Software\Wow6432Node\Microsoft\Windows\CurrentVersion\RunOnce"
            };
            foreach (string subKey in runKeys)
            {
                AddRegistryValues(items, Registry.CurrentUser, subKey, "User", SourceForRegistryPath(subKey), true, "Runs from " + subKey);
                AddRegistryValues(items, Registry.LocalMachine, subKey, "Machine", SourceForRegistryPath(subKey), true, "Runs from " + subKey);
            }
            AddRegistryValues(items, Registry.LocalMachine, @"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon", "Machine", "Winlogon Autostart", false, "Critical logon autostart value", new[] { "Shell", "Userinit", "VMApplet", "Taskman" });
            AddRegistryValues(items, Registry.CurrentUser, @"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon", "User", "Winlogon Autostart", false, "Per-user logon autostart value", new[] { "Shell", "Userinit" });
            AddRegistryValues(items, Registry.LocalMachine, @"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Windows", "Machine", "AppInit DLLs", false, "DLL injection autostart. Disable from Windows security policy or registry with care.", new[] { "AppInit_DLLs", "LoadAppInit_DLLs" });
            AddActiveSetup(items, Registry.LocalMachine, "Machine");
            AddActiveSetup(items, Registry.CurrentUser, "User");
        }

        private static string SourceForRegistryPath(string subKey)
        {
            if (subKey.IndexOf("RunOnce", StringComparison.OrdinalIgnoreCase) >= 0) return "Registry RunOnce";
            if (subKey.IndexOf("RunServices", StringComparison.OrdinalIgnoreCase) >= 0) return "Registry RunServices";
            if (subKey.IndexOf(@"Policies\Explorer\Run", StringComparison.OrdinalIgnoreCase) >= 0) return "Policy Run";
            return "Registry Run";
        }

        private static void AddRegistryValues(List<StartupItem> items, RegistryKey root, string subKey, string scope, string source, bool canEditValue, string status, string[] valueFilter = null)
        {
            try
            {
                using (var key = root.OpenSubKey(subKey, false))
                {
                    if (key == null) return;
                    foreach (var name in key.GetValueNames())
                    {
                        if (valueFilter != null && !valueFilter.Any(x => string.Equals(x, name, StringComparison.OrdinalIgnoreCase))) continue;
                        var value = key.GetValue(name);
                        string cmd = value == null ? "" : value.ToString();
                        if (string.IsNullOrWhiteSpace(cmd)) continue;
                        string encodedName = Convert.ToBase64String(Encoding.UTF8.GetBytes(name ?? ""));
                        string id = "reg|" + scope + "|" + B64(root.Name) + "|" + B64(subKey) + "|" + encodedName;
                        items.Add(new StartupItem { Id = id, Name = string.IsNullOrWhiteSpace(name) ? "(Default)" : name, Source = source, Scope = scope, Command = cmd, Location = root.Name + @"\" + subKey, Enabled = true, CanDisable = canEditValue && (scope == "User" || IsElevated()), IsManaged = false, Status = status });
                    }
                }
            }
            catch (Exception ex) { items.Add(ErrorItem(source, scope, root.Name + @"\" + subKey, ex)); }
        }

        private static void AddActiveSetup(List<StartupItem> items, RegistryKey root, string scope)
        {
            const string subKey = @"SOFTWARE\Microsoft\Active Setup\Installed Components";
            try
            {
                using (var key = root.OpenSubKey(subKey, false))
                {
                    if (key == null) return;
                    foreach (string component in key.GetSubKeyNames())
                    {
                        using (var componentKey = key.OpenSubKey(component, false))
                        {
                            if (componentKey == null) continue;
                            object stub = componentKey.GetValue("StubPath");
                            if (stub == null || string.IsNullOrWhiteSpace(stub.ToString())) continue;
                            object display = componentKey.GetValue(null);
                            string name = string.IsNullOrWhiteSpace(Convert.ToString(display)) ? component : Convert.ToString(display);
                            string location = root.Name + @"\" + subKey + @"\" + component;
                            string id = "active|" + scope + "|" + B64(root.Name) + "|" + B64(subKey + @"\" + component);
                            items.Add(new StartupItem { Id = id, Name = name, Source = "Active Setup", Scope = scope, Command = stub.ToString(), Location = location, Enabled = true, CanDisable = scope == "User" || IsElevated(), IsManaged = false, Status = "Runs once per user profile through Active Setup StubPath" });
                        }
                    }
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("Active Setup", scope, root.Name + @"\" + subKey, ex)); }
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
  $hasLogon=$false; $hasBoot=$false; $hasDelay=$false
  foreach($tr in @($t.Triggers)){
    if($null -eq $tr){ continue }
    $cn = if($tr.CimClass){ [string]$tr.CimClass.CimClassName } else { '' }
    if($cn -like '*LogonTrigger*'){ $hasLogon=$true }
    if($cn -like '*BootTrigger*'){ $hasBoot=$true }
    $delayProp=$tr.PSObject.Properties['Delay']
    if($delayProp -and $delayProp.Value){ $hasDelay=$true }
  }
  if($hasLogon -or $hasBoot){
    $actions = (@($t.Actions) | ForEach-Object { if($_){ (($_.Execute) + ' ' + ($_.Arguments)).Trim() } }) -join ' || '
    $enabled = if($t.Settings.Enabled){'true'}else{'false'}
    $managed = if(($t.TaskPath + $t.TaskName).StartsWith('\MichStartupMaster\')){'true'}else{'false'}
    $k=@(); if($hasLogon){ $k+='logon' }; if($hasBoot){ $k+='boot' }; $triggerKind=$k -join '+'
    ($t.TaskPath + $t.TaskName) + ""`t"" + $enabled + ""`t"" + $t.State + ""`t"" + $hasDelay + ""`t"" + $managed + ""`t"" + $triggerKind + ""`t"" + $actions
  }
}
";
                string encoded = Convert.ToBase64String(Encoding.Unicode.GetBytes(script));
                string output = RunCapture(PowerShellExe(), "-NoProfile -ExecutionPolicy Bypass -EncodedCommand " + encoded);
                foreach (var line in output.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries))
                {
                    string[] p = line.Split(new[] { '\t' }, 7);
                    if (p.Length < 7) continue;
                    string taskName = p[0];
                    bool enabled = p[1].Equals("true", StringComparison.OrdinalIgnoreCase);
                    bool hasDelay = p[3].Equals("true", StringComparison.OrdinalIgnoreCase);
                    bool managed = p[4].Equals("true", StringComparison.OrdinalIgnoreCase);
                    string triggerKind = p[5];
                    string status = (enabled ? "Enabled" : "Disabled") + " " + triggerKind + " startup task" + (hasDelay ? " with delay" : " with no delay");
                    items.Add(new StartupItem { Id = "task|" + taskName, Name = taskName.TrimStart('\\'), Source = "Scheduled Task", Scope = "User/System", Command = p[6], Location = taskName, Enabled = enabled, CanDisable = true, IsManaged = managed, Status = status });
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("Scheduled Task", "System", "Task Scheduler", ex)); }
        }

        private static void AddAutoServices(List<StartupItem> items)
        {
            try
            {
                using (var searcher = new ManagementObjectSearcher("SELECT Name, DisplayName, PathName, StartMode, State FROM Win32_Service"))
                {
                    foreach (ManagementObject mo in searcher.Get())
                    {
                        string startMode = Convert.ToString(mo["StartMode"] ?? "");
                        if (!IsServiceStartupMode(startMode)) continue;
                        string name = Convert.ToString(mo["Name"] ?? "");
                        string displayName = Convert.ToString(mo["DisplayName"] ?? name);
                        string path = Convert.ToString(mo["PathName"] ?? "");
                        string state = Convert.ToString(mo["State"] ?? "");
                        int startValue = ReadServiceStartValue(name, 2);
                        bool enabled = startValue != 4;
                        items.Add(new StartupItem { Id = "service|" + B64(name), Name = string.IsNullOrWhiteSpace(displayName) ? name : displayName, Source = "Windows Service", Scope = "Machine", Command = path, Location = @"HKLM\SYSTEM\CurrentControlSet\Services\" + name, Enabled = enabled, CanDisable = IsElevated(), IsManaged = false, Status = "Service start=" + startMode + " state=" + state + " registryStart=" + startValue });
                    }
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("Windows Service", "Machine", "Win32_Service", ex)); }
        }

        private static void AddAutoDrivers(List<StartupItem> items)
        {
            try
            {
                using (var searcher = new ManagementObjectSearcher("SELECT Name, DisplayName, PathName, StartMode, State FROM Win32_SystemDriver"))
                {
                    foreach (ManagementObject mo in searcher.Get())
                    {
                        string startMode = Convert.ToString(mo["StartMode"] ?? "");
                        if (!IsDriverStartupMode(startMode)) continue;
                        string name = Convert.ToString(mo["Name"] ?? "");
                        string displayName = Convert.ToString(mo["DisplayName"] ?? name);
                        string path = Convert.ToString(mo["PathName"] ?? "");
                        string state = Convert.ToString(mo["State"] ?? "");
                        int startValue = ReadServiceStartValue(name, StartModeToRegistryValue(startMode));
                        bool enabled = startValue != 4;
                        items.Add(new StartupItem { Id = "driver|" + B64(name), Name = string.IsNullOrWhiteSpace(displayName) ? name : displayName, Source = "System Driver", Scope = "Machine", Command = path, Location = @"HKLM\SYSTEM\CurrentControlSet\Services\" + name, Enabled = enabled, CanDisable = IsElevated(), IsManaged = false, Status = "Driver start=" + startMode + " state=" + state + " registryStart=" + startValue });
                    }
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("System Driver", "Machine", "Win32_SystemDriver", ex)); }
        }

        private static void AddStartupApproved(List<StartupItem> items)
        {
            string[] paths = new[]
            {
                @"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run",
                @"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run32",
                @"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder",
                @"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder32"
            };
            foreach (string path in paths)
            {
                AddStartupApprovedValues(items, Registry.CurrentUser, path, "User");
                AddStartupApprovedValues(items, Registry.LocalMachine, path, "Machine");
            }
        }

        private static void AddStartupApprovedValues(List<StartupItem> items, RegistryKey root, string subKey, string scope)
        {
            try
            {
                using (var key = root.OpenSubKey(subKey, false))
                {
                    if (key == null) return;
                    foreach (string name in key.GetValueNames())
                    {
                        byte[] bytes = key.GetValue(name) as byte[];
                        if (bytes == null || bytes.Length == 0) continue;
                        bool disabled = bytes[0] == 0x03 || bytes[0] == 0x05 || bytes[0] == 0x07;
                        if (!disabled) continue;
                        string location = root.Name + @"\" + subKey;
                        string id = "approved|" + scope + "|" + B64(root.Name) + "|" + B64(subKey) + "|" + B64(name);
                        items.Add(new StartupItem { Id = id, Name = name, Source = "Startup Approval", Scope = scope, Command = BitConverter.ToString(bytes), Location = location, Enabled = false, CanDisable = false, IsManaged = false, Status = "Disabled in Explorer StartupApproved metadata; matching Run/folder/task row may also exist" });
                    }
                }
            }
            catch (Exception ex) { items.Add(ErrorItem("Startup Approval", scope, root.Name + @"\" + subKey, ex)); }
        }

        private static bool IsServiceStartupMode(string startMode)
        {
            return string.Equals(startMode, "Auto", StringComparison.OrdinalIgnoreCase) || string.Equals(startMode, "Automatic", StringComparison.OrdinalIgnoreCase);
        }

        private static bool IsDriverStartupMode(string startMode)
        {
            return string.Equals(startMode, "Boot", StringComparison.OrdinalIgnoreCase) || string.Equals(startMode, "System", StringComparison.OrdinalIgnoreCase) || IsServiceStartupMode(startMode);
        }

        public static void Disable(StartupItem item)
        {
            if (item.Id.StartsWith("reg|")) DisableRegistry(item);
            else if (item.Id.StartsWith("active|")) DisableActiveSetup(item);
            else if (item.Id.StartsWith("folder|")) DisableStartupFolder(item);
            else if (item.Id.StartsWith("task|")) RunChecked("schtasks.exe", "/Change /TN " + Q(item.Location) + " /Disable");
            else if (item.Id.StartsWith("service|")) DisableServiceOrDriver(item, "service");
            else if (item.Id.StartsWith("driver|")) DisableServiceOrDriver(item, "driver");
            else throw new InvalidOperationException("Unsupported item: " + item.Id);
            ProtectedDisabledService.Protect(item);
        }

        public static void Enable(StartupItem item)
        {
            ProtectedDisabledService.Unprotect(item);
            if (item.Id.StartsWith("disabled|reg|")) RestoreRegistry(item);
            else if (item.Id.StartsWith("disabled|active|")) RestoreActiveSetup(item);
            else if (item.Id.StartsWith("disabled|folder|")) RestoreStartupFolder(item);
            else if (item.Id.StartsWith("disabled|service|")) RestoreServiceOrDriver(item);
            else if (item.Id.StartsWith("disabled|driver|")) RestoreServiceOrDriver(item);
            else if (item.Id.StartsWith("task|")) RunChecked("schtasks.exe", "/Change /TN " + Q(item.Location) + " /Enable");
            else throw new InvalidOperationException("Unsupported disabled item: " + item.Id);
        }

        private static void DisableRegistry(StartupItem item)
        {
            string[] p = item.Id.Split('|');
            string scope = p[1];
            string rootName;
            string subKey;
            string name;
            if (p.Length >= 5)
            {
                rootName = UnB64(p[2]);
                subKey = UnB64(p[3]);
                name = UnB64(p[4]);
            }
            else
            {
                rootName = scope == "Machine" ? Registry.LocalMachine.Name : Registry.CurrentUser.Name;
                subKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
                name = UnB64(p[2]);
            }
            RegistryKey root = RootFromName(rootName);
            using (var key = root.OpenSubKey(subKey, true))
            {
                if (key == null) throw new InvalidOperationException("Registry startup key missing: " + subKey);
                object value = key.GetValue(name, null, RegistryValueOptions.DoNotExpandEnvironmentNames);
                RegistryValueKind kind = key.GetValueKind(name);
                DisabledStoreService.Add("reg", item.Name, scope, value == null ? "" : value.ToString(), item.Location, root.Name + "\t" + subKey + "\t" + name + "\t" + kind.ToString() + "\t" + item.Source);
                key.DeleteValue(name, false);
            }
        }

        private static void RestoreRegistry(StartupItem item)
        {
            string[] meta = (item.Status ?? "").Split('\t');
            RegistryKey root;
            string subKey;
            string valueName;
            int kindIndex;
            if (meta.Length >= 4 && meta[0].StartsWith("HKEY_", StringComparison.OrdinalIgnoreCase))
            {
                root = RootFromName(meta[0]);
                subKey = meta[1];
                valueName = meta[2];
                kindIndex = 3;
            }
            else
            {
                valueName = meta.Length > 0 ? meta[0] : item.Name;
                root = item.Scope == "Machine" ? Registry.LocalMachine : Registry.CurrentUser;
                subKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
                kindIndex = 1;
            }
            RegistryValueKind kind = RegistryValueKind.String;
            if (meta.Length > kindIndex) { try { kind = (RegistryValueKind)Enum.Parse(typeof(RegistryValueKind), meta[kindIndex], true); } catch { kind = RegistryValueKind.String; } }
            using (var key = root.CreateSubKey(subKey)) key.SetValue(valueName, item.Command ?? "", kind);
            DisabledStoreService.Remove(item.Id);
        }

        private static void DisableActiveSetup(StartupItem item)
        {
            string[] p = item.Id.Split('|');
            if (p.Length < 4) throw new InvalidOperationException("Malformed Active Setup id");
            string scope = p[1];
            RegistryKey root = RootFromName(UnB64(p[2]));
            string subKey = UnB64(p[3]);
            using (var key = root.OpenSubKey(subKey, true))
            {
                if (key == null) throw new InvalidOperationException("Active Setup key missing");
                object value = key.GetValue("StubPath", null, RegistryValueOptions.DoNotExpandEnvironmentNames);
                RegistryValueKind kind = key.GetValueKind("StubPath");
                DisabledStoreService.Add("active", item.Name, scope, value == null ? "" : value.ToString(), item.Location, root.Name + "\t" + subKey + "\tStubPath\t" + kind.ToString());
                key.DeleteValue("StubPath", false);
            }
        }

        private static void RestoreActiveSetup(StartupItem item)
        {
            string[] meta = (item.Status ?? "").Split('\t');
            if (meta.Length < 4) throw new InvalidOperationException("Stored Active Setup metadata is incomplete");
            RegistryKey root = RootFromName(meta[0]);
            RegistryValueKind kind = RegistryValueKind.String;
            try { kind = (RegistryValueKind)Enum.Parse(typeof(RegistryValueKind), meta[3], true); } catch { }
            using (var key = root.CreateSubKey(meta[1])) key.SetValue(meta[2], item.Command ?? "", kind);
            DisabledStoreService.Remove(item.Id);
        }

        private static void DisableServiceOrDriver(StartupItem item, string type)
        {
            string name = UnB64(item.Id.Split('|')[1]);
            int originalStart = ReadServiceStartValue(name, 2);
            SetServiceStartValue(name, 4);
            DisabledStoreService.Add(type, item.Name, item.Scope, item.Command, item.Location, name + "\t" + originalStart.ToString());
        }

        private static void RestoreServiceOrDriver(StartupItem item)
        {
            string[] meta = (item.Status ?? "").Split('\t');
            if (meta.Length < 2) throw new InvalidOperationException("Stored service metadata is incomplete");
            int start;
            if (!int.TryParse(meta[1], out start)) start = 2;
            SetServiceStartValue(meta[0], start);
            DisabledStoreService.Remove(item.Id);
        }

        private static int ReadServiceStartValue(string serviceName, int fallback)
        {
            try
            {
                using (var key = Registry.LocalMachine.OpenSubKey(@"SYSTEM\CurrentControlSet\Services\" + serviceName, false))
                {
                    object value = key == null ? null : key.GetValue("Start");
                    if (value == null) return fallback;
                    return Convert.ToInt32(value);
                }
            }
            catch { return fallback; }
        }

        private static void SetServiceStartValue(string serviceName, int start)
        {
            using (var key = Registry.LocalMachine.OpenSubKey(@"SYSTEM\CurrentControlSet\Services\" + serviceName, true))
            {
                if (key == null) throw new InvalidOperationException("Service registry key missing: " + serviceName);
                key.SetValue("Start", start, RegistryValueKind.DWord);
            }
        }

        private static int StartModeToRegistryValue(string startMode)
        {
            if (string.Equals(startMode, "Boot", StringComparison.OrdinalIgnoreCase)) return 0;
            if (string.Equals(startMode, "System", StringComparison.OrdinalIgnoreCase)) return 1;
            if (IsServiceStartupMode(startMode)) return 2;
            return 3;
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

        public static string AddManagedStartup(string name, string targetPath, string arguments, bool trayMode, bool noDelay)
        {
            if (string.IsNullOrWhiteSpace(name)) throw new ArgumentException("Name is required");
            if (string.IsNullOrWhiteSpace(targetPath) || !File.Exists(targetPath)) throw new FileNotFoundException("Application not found", targetPath);
            if (!IsSupportedStartupTarget(targetPath)) throw new ArgumentException("Choose an executable startup target: .exe, .cmd, .bat, .ps1, or .lnk");
            string safeName = Regex.Replace(name, "[^A-Za-z0-9 _.-]", "").Trim();
            if (safeName.Length == 0) safeName = "StartupApp";
            string execute;
            string actionArgs;
            BuildManagedAction(targetPath, arguments ?? "", trayMode, out execute, out actionArgs);
            string taskLocation = RegisterLogonTaskAt(Program.ManagedTaskRoot + safeName, execute, actionArgs);
            if (trayMode) ProtectedQuietService.ProtectTask(taskLocation, targetPath, arguments ?? "");
            if (!noDelay) { /* Task Scheduler has no explicit Delay either way; this app always uses immediate logon triggers. */ }
            return taskLocation;
        }

        public static string RegisterLogonTaskAt(string fullTaskName, string execute, string arguments)
        {
            string normalized = fullTaskName.StartsWith("\\") ? fullTaskName : Program.ManagedTaskRoot + fullTaskName.Trim('\\');
            int split = normalized.LastIndexOf('\\');
            string path = split >= 0 ? normalized.Substring(0, split + 1) : Program.ManagedTaskRoot;
            string taskName = split >= 0 ? normalized.Substring(split + 1) : normalized.Trim('\\');
            if (string.IsNullOrWhiteSpace(path)) path = Program.ManagedTaskRoot;
            if (string.IsNullOrWhiteSpace(taskName)) throw new ArgumentException("Task name is required");
            if (string.IsNullOrWhiteSpace(execute) || !File.Exists(execute)) throw new FileNotFoundException("Startup executable was not found", execute);
            string script =
                "$ErrorActionPreference='Stop';" +
                "function D($s){[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s))};" +
                "$path = D '" + B64(path) + "';" +
                "$taskName = D '" + B64(taskName) + "';" +
                "$execute = D '" + B64(execute) + "';" +
                "$arguments = D '" + B64(arguments ?? "") + "';" +
                "if(-not (Test-Path -LiteralPath $execute)){throw ('Startup executable was not found: ' + $execute)};" +
                "$action=if([string]::IsNullOrWhiteSpace($arguments)){New-ScheduledTaskAction -Execute $execute}else{New-ScheduledTaskAction -Execute $execute -Argument $arguments};" +
                "$trigger=New-ScheduledTaskTrigger -AtLogOn;" +
                "$principal=New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType Interactive -RunLevel Limited;" +
                "$settings=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit (New-TimeSpan -Minutes 0);" +
                "Register-ScheduledTask -TaskPath $path -TaskName $taskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null;";
            RunPowerShellScript(script);
            return path + taskName;
        }

        private static string B64(string s) { return Convert.ToBase64String(Encoding.UTF8.GetBytes(s ?? "")); }
        private static string UnB64(string s) { try { return Encoding.UTF8.GetString(Convert.FromBase64String(s)); } catch { return ""; } }

        private static RegistryKey RootFromName(string rootName)
        {
            if (string.Equals(rootName, Registry.LocalMachine.Name, StringComparison.OrdinalIgnoreCase) || string.Equals(rootName, "HKLM", StringComparison.OrdinalIgnoreCase)) return Registry.LocalMachine;
            if (string.Equals(rootName, Registry.CurrentUser.Name, StringComparison.OrdinalIgnoreCase) || string.Equals(rootName, "HKCU", StringComparison.OrdinalIgnoreCase)) return Registry.CurrentUser;
            throw new InvalidOperationException("Unsupported registry root: " + rootName);
        }

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
            ProtectedQuietService.UnprotectTask(tn);
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
            if (item.PopupLabel() == "N/A") throw new InvalidOperationException("Popup control applies to app/task/folder/registry launch commands, not service, driver, Winlogon, AppInit, or Active Setup rows.");
            string target, arguments;
            ResolveLaunchTarget(item, out target, out arguments);
            string execute = popupEnabled ? target : Process.GetCurrentProcess().MainModule.FileName;
            string actionArgs = popupEnabled ? (IsSelfTarget(target) ? "" : (arguments ?? "")) : (IsSelfTarget(target) ? "--start-in-tray" : "--tray-run " + Convert.ToBase64String(Encoding.UTF8.GetBytes(target + "\n" + (arguments ?? ""))));

            if (item.Id.StartsWith("task|") && item.IsManaged)
            {
                RegisterLogonTaskAt(item.Location, execute, actionArgs);
                if (popupEnabled) ProtectedQuietService.UnprotectTask(item.Location);
                else ProtectedQuietService.ProtectTask(item.Location, target, arguments ?? "");
                return;
            }

            if (item.Enabled && item.CanDisable) Disable(item);
            else if (item.Enabled && !item.CanDisable) throw new InvalidOperationException("This startup source is read-only here. Select its matching Registry Run, Startup Folder, or Scheduled Task row if Windows exposes one.");

            string createdTask = AddManagedStartup(item.Name, target, arguments, !popupEnabled, true);
            if (!popupEnabled) ProtectedQuietService.ProtectTask(createdTask, target, arguments ?? "");
        }

        public static string EditStartup(StartupItem item, string name, string targetPath, string arguments, bool trayMode)
        {
            if (item == null) throw new ArgumentException("No startup item selected");
            if (string.IsNullOrWhiteSpace(name)) throw new ArgumentException("Name is required");
            if (string.IsNullOrWhiteSpace(targetPath) || !File.Exists(targetPath)) throw new FileNotFoundException("Application not found", targetPath);
            if (!IsSupportedStartupTarget(targetPath)) throw new ArgumentException("Choose an executable startup target: .exe, .cmd, .bat, .ps1, or .lnk");
            if (item.Id.StartsWith("task|") && item.IsManaged)
            {
                string execute;
                string actionArgs;
                BuildManagedAction(targetPath, arguments ?? "", trayMode, out execute, out actionArgs);
                RegisterLogonTaskAt(item.Location, execute, actionArgs);
                if (trayMode) ProtectedQuietService.ProtectTask(item.Location, targetPath, arguments ?? "");
                else ProtectedQuietService.UnprotectTask(item.Location);
                return item.Location;
            }
            if (item.PopupLabel() == "N/A") throw new InvalidOperationException("This startup source cannot be edited as an application launch. Services, drivers, Winlogon, AppInit, and Active Setup rows should be changed from their owning tool.");
            if (item.Enabled && item.CanDisable) Disable(item);
            else if (item.Enabled && !item.CanDisable) throw new InvalidOperationException("This startup source is read-only here. Run elevated or choose its matching Registry Run, Startup Folder, or Scheduled Task row.");
            return AddManagedStartup(name, targetPath, arguments ?? "", trayMode, true);
        }

        private static void BuildManagedAction(string targetPath, string arguments, bool trayMode, out string execute, out string actionArgs)
        {
            if (trayMode)
            {
                execute = Process.GetCurrentProcess().MainModule.FileName;
                if (IsSelfTarget(targetPath)) actionArgs = "--start-in-tray";
                else actionArgs = "--tray-run " + Convert.ToBase64String(Encoding.UTF8.GetBytes(targetPath + "\n" + (arguments ?? "")));
            }
            else BuildDirectAction(targetPath, arguments ?? "", out execute, out actionArgs);
        }

        public static bool IsSupportedStartupTarget(string path)
        {
            string ext = Path.GetExtension(path ?? "").ToLowerInvariant();
            return ext == ".exe" || ext == ".cmd" || ext == ".bat" || ext == ".ps1" || ext == ".lnk";
        }

        public static void BuildDirectAction(string targetPath, string arguments, out string execute, out string actionArgs)
        {
            string ext = Path.GetExtension(targetPath ?? "").ToLowerInvariant();
            if (ext == ".ps1")
            {
                execute = PowerShellExe();
                actionArgs = "-NoProfile -ExecutionPolicy Bypass -File " + WinArg(targetPath) + AppendArgs(arguments);
            }
            else if (ext == ".cmd" || ext == ".bat")
            {
                execute = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), @"System32\cmd.exe");
                actionArgs = "/d /c " + WinArg(targetPath) + AppendArgs(arguments);
            }
            else if (ext == ".lnk")
            {
                execute = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "explorer.exe");
                actionArgs = WinArg(targetPath) + AppendArgs(arguments);
            }
            else
            {
                execute = targetPath;
                actionArgs = arguments ?? "";
            }
        }

        private static string AppendArgs(string arguments) { return string.IsNullOrWhiteSpace(arguments) ? "" : " " + arguments.Trim(); }
        private static string WinArg(string value) { return "\"" + (value ?? "").Replace("\"", "\\\"") + "\""; }

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
            if (!File.Exists(targetPath) || !IsSupportedStartupTarget(targetPath)) throw new FileNotFoundException("Resolved startup target is not a supported startup target", targetPath);
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
                sb.Append("{\"name\":\"").Append(Esc(x.Name)).Append("\",\"appName\":\"").Append(Esc(x.HumanName())).Append("\",\"source\":\"").Append(Esc(x.Source)).Append("\",\"scope\":\"").Append(Esc(x.Scope)).Append("\",\"location\":\"").Append(Esc(x.Location)).Append("\",\"enabled\":").Append(x.Enabled ? "true" : "false").Append(",\"canDisable\":").Append(x.CanDisable ? "true" : "false").Append(",\"command\":\"").Append(Esc(x.Command)).Append("\",\"status\":\"").Append(Esc(x.Status)).Append("\",\"popup\":\"").Append(x.PopupLabel()).Append("\",\"risk\":\"").Append(Esc(x.RiskLevel())).Append("\",\"riskLabel\":\"").Append(Esc(x.RiskLabel())).Append("\",\"riskReason\":\"").Append(Esc(x.RiskReason())).Append("\",\"advice\":\"").Append(Esc(x.AdviceLevel())).Append("\",\"adviceLabel\":\"").Append(Esc(x.AdviceLabel())).Append("\",\"adviceReason\":\"").Append(Esc(x.AdviceReason())).Append("\"}");
            }
            sb.Append("]"); return sb.ToString();
        }

        private static string Esc(string s) { return (s ?? "").Replace("\\", "\\\\").Replace("\"", "\\\"").Replace("\r", " ").Replace("\n", " "); }
        private static StartupItem ErrorItem(string source, string scope, string location, Exception ex) { return new StartupItem { Id = "error|" + source + "|" + location, Name = "Scan warning", Source = source, Scope = scope, Location = location, Command = ex.Message, Enabled = false, CanDisable = false, Status = "Read failed" }; }
        private static string Q(string s) { return "\"" + (s ?? "").Replace("\"", "\\\"") + "\""; }

        private static string PowerShellExe()
        {
            string full = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), @"System32\WindowsPowerShell\v1.0\powershell.exe");
            return File.Exists(full) ? full : "powershell.exe";
        }

        private static void RunPowerShellScript(string script)
        {
            string temp = Path.Combine(Program.AppData, "task-register-" + Guid.NewGuid().ToString("N") + ".ps1");
            Directory.CreateDirectory(Program.AppData);
            File.WriteAllText(temp, script, new UTF8Encoding(false));
            try { RunChecked(PowerShellExe(), "-NoProfile -ExecutionPolicy Bypass -File " + Q(temp)); }
            finally { try { File.Delete(temp); } catch { } }
        }

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
                string type = p[1];
                list.Add(new StartupItem { Id = p[0], Name = UnB64(p[2]), Source = SourceName(type, UnB64(p[6])), Scope = UnB64(p[3]), Command = UnB64(p[4]), Location = UnB64(p[5]), Status = UnB64(p[6]), Enabled = false, CanDisable = true, IsManaged = true });
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
        private static string SourceName(string type, string status)
        {
            if (type == "folder") return "Startup Folder";
            if (type == "service") return "Windows Service";
            if (type == "driver") return "System Driver";
            if (type == "active") return "Active Setup";
            if (type == "reg")
            {
                string[] meta = (status ?? "").Split('\t');
                if (meta.Length >= 5 && !string.IsNullOrWhiteSpace(meta[4])) return meta[4];
                return "Registry Run";
            }
            return type;
        }
    }

    internal static class ProtectedDisabledService
    {
        private static string Key(StartupItem item)
        {
            if (item == null) return "";
            if (item.Id.StartsWith("disabled|reg|"))
            {
                string[] meta = (item.Status ?? "").Split('\t');
                if (meta.Length >= 4 && meta[0].StartsWith("HKEY_", StringComparison.OrdinalIgnoreCase)) return "reg|" + item.Scope + "|" + meta[0] + "|" + meta[1] + "|" + meta[2];
                return "reg|" + item.Scope + "|" + (meta.Length > 0 ? meta[0] : item.Name);
            }
            if (item.Id.StartsWith("disabled|active|"))
            {
                string[] meta = (item.Status ?? "").Split('\t');
                if (meta.Length >= 2) return "active|" + item.Scope + "|" + meta[0] + "|" + meta[1];
            }
            if (item.Id.StartsWith("disabled|service|") || item.Id.StartsWith("disabled|driver|")) return item.Id.Split('|')[1] + "|" + (item.Status ?? "").Split('\t')[0];
            if (item.Id.StartsWith("reg|")) return item.Id;
            if (item.Id.StartsWith("active|")) return item.Id;
            if (item.Id.StartsWith("disabled|folder|")) return "folder|" + item.Scope + "|" + item.Status;
            if (item.Id.StartsWith("folder|")) return item.Id;
            if (item.Id.StartsWith("task|")) return "task|" + item.Location;
            if (item.Id.StartsWith("service|") || item.Id.StartsWith("driver|")) return item.Id;
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
                string[] meta = (r.Status ?? "").Split('\t');
                RegistryKey root;
                string subKey;
                string valueName;
                if (meta.Length >= 4 && meta[0].StartsWith("HKEY_", StringComparison.OrdinalIgnoreCase))
                {
                    root = RootFromName(meta[0]);
                    subKey = meta[1];
                    valueName = meta[2];
                }
                else
                {
                    valueName = meta.Length > 0 ? meta[0] : r.Name;
                    root = (r.Scope ?? "").Equals("Machine", StringComparison.OrdinalIgnoreCase) ? Registry.LocalMachine : Registry.CurrentUser;
                    subKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
                }
                if (string.IsNullOrWhiteSpace(valueName)) valueName = r.Name;
                using (var key = root.OpenSubKey(subKey, true))
                {
                    if (key != null && key.GetValueNames().Any(n => string.Equals(n, valueName, StringComparison.OrdinalIgnoreCase))) { key.DeleteValue(valueName, false); return true; }
                }
                return false;
            }
            if (r.Key.StartsWith("active|", StringComparison.OrdinalIgnoreCase) || r.Type == "Active Setup")
            {
                string[] meta = (r.Status ?? "").Split('\t');
                if (meta.Length < 3) return false;
                RegistryKey root = RootFromName(meta[0]);
                using (var key = root.OpenSubKey(meta[1], true))
                {
                    if (key != null && key.GetValueNames().Any(n => string.Equals(n, meta[2], StringComparison.OrdinalIgnoreCase))) { key.DeleteValue(meta[2], false); return true; }
                }
                return false;
            }
            if (r.Key.StartsWith("service|", StringComparison.OrdinalIgnoreCase) || r.Key.StartsWith("driver|", StringComparison.OrdinalIgnoreCase) || r.Type == "Windows Service" || r.Type == "System Driver")
            {
                string serviceName = (r.Status ?? "").Split('\t')[0];
                if (string.IsNullOrWhiteSpace(serviceName)) return false;
                using (var key = Registry.LocalMachine.OpenSubKey(@"SYSTEM\CurrentControlSet\Services\" + serviceName, true))
                {
                    if (key == null) return false;
                    object current = key.GetValue("Start");
                    if (current != null && Convert.ToInt32(current) == 4) return false;
                    key.SetValue("Start", 4, RegistryValueKind.DWord);
                    return true;
                }
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
        private static RegistryKey RootFromName(string rootName)
        {
            if (string.Equals(rootName, Registry.LocalMachine.Name, StringComparison.OrdinalIgnoreCase) || string.Equals(rootName, "HKLM", StringComparison.OrdinalIgnoreCase)) return Registry.LocalMachine;
            if (string.Equals(rootName, Registry.CurrentUser.Name, StringComparison.OrdinalIgnoreCase) || string.Equals(rootName, "HKCU", StringComparison.OrdinalIgnoreCase)) return Registry.CurrentUser;
            throw new InvalidOperationException("Unsupported registry root: " + rootName);
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

    internal static class ProtectedQuietService
    {
        public static void ProtectTask(string taskLocation, string targetPath, string arguments)
        {
            if (string.IsNullOrWhiteSpace(taskLocation) || string.IsNullOrWhiteSpace(targetPath)) return;
            Directory.CreateDirectory(Program.AppData);
            var rows = LoadRows().Where(r => !string.Equals(r.TaskLocation, taskLocation, StringComparison.OrdinalIgnoreCase)).ToList();
            rows.Add(new Row { TaskLocation = taskLocation, TargetPath = targetPath, Arguments = arguments ?? "" });
            SaveRows(rows);
        }

        public static void UnprotectTask(string taskLocation)
        {
            if (string.IsNullOrWhiteSpace(taskLocation) || !File.Exists(Program.ProtectedQuietStore)) return;
            SaveRows(LoadRows().Where(r => !string.Equals(r.TaskLocation, taskLocation, StringComparison.OrdinalIgnoreCase)).ToList());
        }

        public static string EnforceProtected()
        {
            int actions = 0, failures = 0;
            foreach (var row in LoadRows())
            {
                try
                {
                    if (File.Exists(row.TargetPath))
                    {
                        string args = StartupService.CommandUsesTrayWrapper(row.TargetPath + " " + row.Arguments) ? row.Arguments : QuietArguments(row.TargetPath, row.Arguments);
                        StartupService.RegisterLogonTaskAt(row.TaskLocation, Process.GetCurrentProcess().MainModule.FileName, args);
                        actions++;
                    }
                }
                catch { failures++; }
            }
            return "ENFORCE_QUIET protected=" + LoadRows().Count + " actions=" + actions + " failures=" + failures;
        }

        private static string QuietArguments(string targetPath, string arguments)
        {
            try
            {
                string self = Path.GetFullPath(Process.GetCurrentProcess().MainModule.FileName).TrimEnd('\\');
                string target = Path.GetFullPath(targetPath ?? "").TrimEnd('\\');
                if (string.Equals(self, target, StringComparison.OrdinalIgnoreCase)) return "--start-in-tray";
            }
            catch { }
            return "--tray-run " + Convert.ToBase64String(Encoding.UTF8.GetBytes((targetPath ?? "") + "\n" + (arguments ?? "")));
        }

        private static List<Row> LoadRows()
        {
            var rows = new List<Row>();
            if (!File.Exists(Program.ProtectedQuietStore)) return rows;
            foreach (string line in File.ReadAllLines(Program.ProtectedQuietStore))
            {
                string[] p = line.Split('\t');
                if (p.Length < 3) continue;
                rows.Add(new Row { TaskLocation = UnB64(p[0]), TargetPath = UnB64(p[1]), Arguments = UnB64(p[2]) });
            }
            return rows;
        }

        private static void SaveRows(List<Row> rows)
        {
            Directory.CreateDirectory(Program.AppData);
            File.WriteAllLines(Program.ProtectedQuietStore, rows.Select(r => string.Join("\t", new[] { B64(r.TaskLocation), B64(r.TargetPath), B64(r.Arguments) })), Encoding.UTF8);
        }

        private static string B64(string s) { return Convert.ToBase64String(Encoding.UTF8.GetBytes(s ?? "")); }
        private static string UnB64(string s) { try { return Encoding.UTF8.GetString(Convert.FromBase64String(s)); } catch { return ""; } }
        private sealed class Row { public string TaskLocation, TargetPath, Arguments; }
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
                if (!File.Exists(full) || !StartupService.IsSupportedStartupTarget(full)) return;
                string execute, actionArgs;
                StartupService.BuildDirectAction(full, targetArgs, out execute, out actionArgs);
                var psi = new ProcessStartInfo(execute, actionArgs)
                {
                    UseShellExecute = false,
                    CreateNoWindow = true,
                    WindowStyle = ProcessWindowStyle.Normal,
                    WorkingDirectory = Directory.Exists(Path.GetDirectoryName(full)) ? Path.GetDirectoryName(full) : Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)
                };
                Process.Start(psi);
            }
            catch { }
        }

        private static void HideProcessWindows(Process child, TimeSpan duration)
        {
            return;
        }

        private static void HideWindowsForProcessTree(int rootPid)
        {
            return;
        }

        private static IEnumerable<int> ChildProcessIds(int rootPid)
        {
            var ids = new List<int>();
            try
            {
                using (var searcher = new ManagementObjectSearcher("SELECT ProcessId FROM Win32_Process WHERE ParentProcessId=" + rootPid))
                {
                    foreach (ManagementObject mo in searcher.Get())
                    {
                        int pid = Convert.ToInt32(mo["ProcessId"]);
                        ids.Add(pid);
                        ids.AddRange(ChildProcessIds(pid));
                    }
                }
            }
            catch { }
            return ids;
        }

        private static void HideWindowsForPid(int pid)
        {
            return;
        }
    }

    internal sealed class MainForm : Form
    {
        private List<StartupItem> _items = new List<StartupItem>();
        private ListView _list;
        private TextBox _search;
        private Label _summary, _visibleValue, _enabledValue, _disabledValue, _reviewValue, _managedValue, _hint;
        private Button _refresh, _disable, _enable, _add, _editSelected, _deleteManaged, _clearSearch, _showAll, _showRisky, _showCleanup, _showDisabled, _quietSelected, _protectNow, _enforceNow, _openFolders;
        private NotifyIcon _tray;
        private Timer _guardTimer;
        private bool _reallyExit;
        private bool _isRefreshing;
        private int _refreshVersion;
        private string _filterMode = "All";
        private readonly bool _startInTray;
        private readonly Color Bg = Color.FromArgb(7, 12, 18), Surface = Color.FromArgb(16, 28, 36), Surface2 = Color.FromArgb(24, 42, 52), Accent = Color.FromArgb(20, 184, 166), TextMain = Color.FromArgb(243, 250, 247), Muted = Color.FromArgb(158, 176, 173), Good = Color.FromArgb(52, 211, 153), Danger = Color.FromArgb(239, 68, 68), Warn = Color.FromArgb(245, 158, 11), Steel = Color.FromArgb(59, 130, 246);

        public static string UiContractJson()
        {
            return "{\"columns\":[\"Status\",\"Application\",\"Startup entry\",\"Source\",\"Risk\",\"Cleanup\",\"Popup\",\"Location\",\"Launch command\"],\"popupEnabledLabel\":\"Enabled\",\"popupDisabledLabel\":\"Disabled\",\"popupNotApplicableLabel\":\"N/A\",\"oneClickPopupToggle\":true,\"trayIcon\":true,\"trayDoubleClickOpens\":true,\"startInTrayArgument\":\"--start-in-tray\",\"asyncRefresh\":true,\"humanReadableNames\":true,\"greenCleanupAdvice\":true,\"contextMenu\":true,\"keyboardShortcuts\":true,\"filters\":[\"All\",\"High risk\",\"Suggested cleanup\",\"Disabled\"],\"tools\":[\"Add startup\",\"Edit startup\",\"Remove startup\",\"Restore startup\",\"Make quiet\",\"Launch now\",\"Open location\",\"Copy command\",\"Protect disabled\",\"Enforce now\",\"Open startup folders\"]}";
        }

        public MainForm(bool startInTray = false)
        {
            _startInTray = startInTray;
            Text = "Mich Startup Master - Windows Boot Control";
            Width = 1440; Height = 900; MinimumSize = new Size(1160, 760);
            BackColor = Bg; Font = new Font("Segoe UI", 10f); DoubleBuffered = true; Icon = Program.AppIcon; KeyPreview = true;
            BuildUi(); BuildTray();
            Load += (s, e) => RefreshItems();
            _guardTimer = new Timer { Interval = 30000 };
            _guardTimer.Tick += (s, e) => RunGuardsAsync(false);
            _guardTimer.Start();
            FormClosing += OnClosingToTray;
            Resize += (s, e) => { if (WindowState == FormWindowState.Minimized) HideToTray(); };
            KeyDown += MainFormKeyDown;
            Shown += (s, e) => { if (_startInTray) BeginInvoke(new Action(HideToTray)); };
        }

        protected override void OnPaint(PaintEventArgs e)
        {
            using (var b = new LinearGradientBrush(ClientRectangle, Color.FromArgb(4, 12, 18), Color.FromArgb(22, 48, 42), 28f)) e.Graphics.FillRectangle(b, ClientRectangle);
            using (var glow = new SolidBrush(Color.FromArgb(42, 20, 184, 166))) e.Graphics.FillEllipse(glow, Width - 420, -180, 620, 420);
            using (var glow2 = new SolidBrush(Color.FromArgb(28, 245, 158, 11))) e.Graphics.FillEllipse(glow2, -180, Height - 260, 420, 320);
            base.OnPaint(e);
        }

        private void BuildUi()
        {
            var hero = Card(new Rectangle(28, 24, Width - 72, 158));
            hero.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right;
            Controls.Add(hero);
            var title = new Label { Text = "Startup Master", ForeColor = TextMain, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 30f), AutoSize = true, Location = new Point(26, 18) };
            var sub = new Label { Text = "One command center for every Windows startup route: tasks, services, drivers, registry, folders, quiet tray launch, and protected disabled state.", ForeColor = Muted, BackColor = Color.Transparent, Font = new Font("Segoe UI", 11.5f), AutoSize = false, Width = 900, Height = 46, Location = new Point(30, 72) };
            _summary = new Label { ForeColor = Color.White, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 10.5f), AutoSize = true, Location = new Point(300, 120) };
            hero.Controls.Add(title); hero.Controls.Add(sub); hero.Controls.Add(_summary);
            _add = Button("+ Add startup", Accent, 170); _add.Location = new Point(30, 112); _add.Click += (s, e) => AddBootApp(); hero.Controls.Add(_add);
            _refresh = Button("Refresh inventory", Steel, 160); _refresh.Location = new Point(hero.Width - 190, 104); _refresh.Anchor = AnchorStyles.Top | AnchorStyles.Right; _refresh.Click += (s, e) => RefreshItems(); hero.Controls.Add(_refresh);

            int cardTop = 202, cardW = 194, gap = 14;
            _visibleValue = MetricCard("Visible", "startup items in view", Color.FromArgb(129, 140, 248), 28 + (cardW + gap) * 0, cardTop, cardW);
            _enabledValue = MetricCard("Enabled", "will run at boot", Good, 28 + (cardW + gap) * 1, cardTop, cardW);
            _disabledValue = MetricCard("Disabled", "kept from startup", Danger, 28 + (cardW + gap) * 2, cardTop, cardW);
            _reviewValue = MetricCard("Cleanup", "green suggestions", Good, 28 + (cardW + gap) * 3, cardTop, cardW);
            _managedValue = MetricCard("Managed", "created here", Accent, 28 + (cardW + gap) * 4, cardTop, cardW);

            var toolbar = Card(new Rectangle(28, 302, Width - 72, 112));
            toolbar.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right; Controls.Add(toolbar);
            var searchLabel = new Label { Text = "Search", ForeColor = Muted, BackColor = Color.Transparent, Font = new Font("Segoe UI Semibold", 9f), Location = new Point(18, 8), AutoSize = true }; toolbar.Controls.Add(searchLabel);
            _search = StyledTextBox(); _search.Location = new Point(18, 30); _search.Width = 390; _search.TextChanged += (s, e) => RenderList(); toolbar.Controls.Add(_search);
            _clearSearch = Button("Clear", Surface2, 82); _clearSearch.Location = new Point(440, 26); _clearSearch.Height = 32; _clearSearch.Click += (s, e) => { _search.Text = ""; }; toolbar.Controls.Add(_clearSearch);
            _showAll = Button("All", Accent, 72); _showAll.Location = new Point(18, 70); _showAll.Click += (s, e) => SetFilter("All"); toolbar.Controls.Add(_showAll);
            _showRisky = Button("High risk", Danger, 104); _showRisky.Location = new Point(100, 70); _showRisky.Click += (s, e) => SetFilter("Risky"); toolbar.Controls.Add(_showRisky);
            _showCleanup = Button("Suggested", Good, 110); _showCleanup.Location = new Point(214, 70); _showCleanup.Click += (s, e) => SetFilter("Cleanup"); toolbar.Controls.Add(_showCleanup);
            _showDisabled = Button("Disabled", Surface2, 104); _showDisabled.Location = new Point(334, 70); _showDisabled.Click += (s, e) => SetFilter("Disabled"); toolbar.Controls.Add(_showDisabled);
            _editSelected = Button("Edit", Steel, 82); _editSelected.Location = new Point(toolbar.Width - 830, 18); _editSelected.Anchor = AnchorStyles.Top | AnchorStyles.Right; _editSelected.Click += (s, e) => EditSelected(); toolbar.Controls.Add(_editSelected);
            _quietSelected = Button("Make quiet", Accent, 120); _quietSelected.Location = new Point(toolbar.Width - 738, 18); _quietSelected.Anchor = AnchorStyles.Top | AnchorStyles.Right; _quietSelected.Click += (s, e) => MakeSelectedQuiet(); toolbar.Controls.Add(_quietSelected);
            _disable = Button("Remove", Danger, 110); _disable.Location = new Point(toolbar.Width - 608, 18); _disable.Anchor = AnchorStyles.Top | AnchorStyles.Right; _disable.Click += (s, e) => DisableSelected(); toolbar.Controls.Add(_disable);
            _enable = Button("Restore", Good, 105); _enable.Location = new Point(toolbar.Width - 488, 18); _enable.Anchor = AnchorStyles.Top | AnchorStyles.Right; _enable.Click += (s, e) => EnableSelected(); toolbar.Controls.Add(_enable);
            _deleteManaged = Button("Delete task", Warn, 115); _deleteManaged.Location = new Point(toolbar.Width - 373, 18); _deleteManaged.Anchor = AnchorStyles.Top | AnchorStyles.Right; _deleteManaged.Click += (s, e) => DeleteManaged(); toolbar.Controls.Add(_deleteManaged);
            _protectNow = Button("Protect disabled", Surface2, 135); _protectNow.Location = new Point(toolbar.Width - 270, 18); _protectNow.Anchor = AnchorStyles.Top | AnchorStyles.Right; _protectNow.Click += (s, e) => ProtectDisabledNow(); toolbar.Controls.Add(_protectNow);
            _enforceNow = Button("Enforce now", Steel, 112); _enforceNow.Location = new Point(toolbar.Width - 130, 18); _enforceNow.Anchor = AnchorStyles.Top | AnchorStyles.Right; _enforceNow.Click += (s, e) => RunGuardsAsync(true); toolbar.Controls.Add(_enforceNow);
            _openFolders = Button("Open startup folders", Surface2, 160); _openFolders.Location = new Point(toolbar.Width - 190, 66); _openFolders.Anchor = AnchorStyles.Top | AnchorStyles.Right; _openFolders.Click += (s, e) => OpenStartupFolders(); toolbar.Controls.Add(_openFolders);

            var listCard = Card(new Rectangle(28, 432, Width - 72, Height - 488));
            listCard.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right | AnchorStyles.Bottom; Controls.Add(listCard);
            _hint = new Label { Text = "Loading startup inventory without blocking the window...", ForeColor = Muted, BackColor = Color.Transparent, Font = new Font("Segoe UI", 9.5f), Location = new Point(18, 12), AutoSize = true }; listCard.Controls.Add(_hint);
            _list = new ListView { Location = new Point(18, 42), Size = new Size(listCard.Width - 36, listCard.Height - 60), Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right | AnchorStyles.Bottom, View = View.Details, FullRowSelect = true, BorderStyle = BorderStyle.None, BackColor = Color.FromArgb(12, 18, 34), ForeColor = TextMain, Font = new Font("Segoe UI", 9.7f), HideSelection = false, OwnerDraw = true };
            _list.SmallImageList = new ImageList { ImageSize = new Size(1, 34) };
            _list.Columns.Add("Status", 105); _list.Columns.Add("Application", 250); _list.Columns.Add("Startup entry", 220); _list.Columns.Add("Source", 145); _list.Columns.Add("Risk", 112); _list.Columns.Add("Cleanup", 112); _list.Columns.Add("Popup", 112); _list.Columns.Add("Location", 250); _list.Columns.Add("Launch command", 430);
            _list.DrawColumnHeader += (s, e) => { using (var b = new SolidBrush(Surface2)) e.Graphics.FillRectangle(b, e.Bounds); TextRenderer.DrawText(e.Graphics, e.Header.Text, new Font(Font, FontStyle.Bold), new Rectangle(e.Bounds.X + 8, e.Bounds.Y, e.Bounds.Width, e.Bounds.Height), Color.White, TextFormatFlags.VerticalCenter | TextFormatFlags.Left); };
            _list.DrawSubItem += DrawSubItem; _list.SelectedIndexChanged += (s, e) => UpdateButtons(); _list.MouseDown += SelectListItemOnRightClick; _list.MouseUp += ListMouseUpPopupToggle; _list.DoubleClick += (s, e) => EditSelected();
            _list.ContextMenuStrip = BuildListContextMenu();
            _list.Resize += (s, e) => { if (_list.Columns.Count > 8) _list.Columns[8].Width = Math.Max(300, _list.Width - 1306); };
            listCard.Controls.Add(_list);
        }

        private ContextMenuStrip BuildListContextMenu()
        {
            var menu = new ContextMenuStrip();
            menu.Items.Add("Edit startup", null, (s, e) => EditSelected());
            menu.Items.Add("Remove from startup", null, (s, e) => DisableSelected());
            menu.Items.Add("Restore startup", null, (s, e) => EnableSelected());
            menu.Items.Add(new ToolStripSeparator());
            menu.Items.Add("Make quiet", null, (s, e) => MakeSelectedQuiet());
            menu.Items.Add("Launch now", null, (s, e) => LaunchSelectedNow());
            menu.Items.Add("Open location", null, (s, e) => OpenSelectedLocation());
            menu.Items.Add("Copy launch command", null, (s, e) => CopySelectedCommand());
            menu.Items.Add(new ToolStripSeparator());
            menu.Items.Add("Refresh", null, (s, e) => RefreshItems());
            return menu;
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
            if (!selected && item.RiskLevel() == "Critical") row = e.ItemIndex % 2 == 0 ? Color.FromArgb(54, 16, 22) : Color.FromArgb(68, 20, 28);
            else if (!selected && item.AdviceLevel() == "Cleanup") row = e.ItemIndex % 2 == 0 ? Color.FromArgb(10, 44, 34) : Color.FromArgb(12, 54, 41);
            using (var b = new SolidBrush(row)) e.Graphics.FillRectangle(b, e.Bounds);
            if (e.ColumnIndex == 6) { DrawPopupToggle(e.Graphics, e.Bounds, item); return; }
            Color c = TextMain; string text = e.SubItem.Text;
            if (e.ColumnIndex == 0) { c = item.Enabled ? Good : Danger; text = item.Enabled ? "● Enabled" : "● Disabled"; }
            if (e.ColumnIndex == 4) c = item.RiskLevel() == "Critical" ? Color.FromArgb(255, 180, 180) : (item.RiskLevel() == "Review" ? Warn : Good);
            if (e.ColumnIndex == 5) c = item.AdviceLevel() == "Cleanup" ? Good : Muted;
            if (e.ColumnIndex == 7 || e.ColumnIndex == 8) c = Muted;
            TextRenderer.DrawText(e.Graphics, text, _list.Font, new Rectangle(e.Bounds.X + 10, e.Bounds.Y, e.Bounds.Width - 12, e.Bounds.Height), c, TextFormatFlags.VerticalCenter | TextFormatFlags.EndEllipsis | TextFormatFlags.Left);
        }

        private void DrawPopupToggle(Graphics g, Rectangle bounds, StartupItem item)
        {
            bool popupEnabled = item.PopupEnabled();
            Rectangle r = PopupButtonRect(bounds);
            bool notApplicable = item.PopupLabel() == "N/A";
            Color color = notApplicable ? Surface2 : (popupEnabled ? Warn : Good);
            string text = notApplicable ? "N/A" : (popupEnabled ? "Enabled" : "Disabled");
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
            var trayMenu = new ContextMenuStrip();
            trayMenu.Items.Add("Open Startup Master", null, (s, e) => OpenFromTray());
            trayMenu.Items.Add("Refresh inventory", null, (s, e) => RefreshItems());
            trayMenu.Items.Add("Enforce quiet + disabled guards", null, (s, e) => RunGuardsAsync(true));
            trayMenu.Items.Add("Exit", null, (s, e) => { _reallyExit = true; if (_tray != null) { _tray.Visible = false; _tray.Dispose(); } Application.Exit(); });
            _tray.ContextMenuStrip = trayMenu;
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
        private void RefreshItems()
        {
            if (_isRefreshing) return;
            _isRefreshing = true;
            int version = ++_refreshVersion;
            Cursor = Cursors.AppStarting;
            SetBusy(true, "Refreshing startup inventory in the background...");
            Task.Run(() =>
            {
                ProtectedDisabledService.EnforceProtected();
                ProtectedQuietService.EnforceProtected();
                var scanned = StartupService.ScanAll();
                ProtectedDisabledService.ProtectCurrentDisabled();
                return scanned;
            }).ContinueWith(t =>
            {
                if (IsDisposed) return;
                BeginInvoke(new Action(() =>
                {
                    if (version != _refreshVersion) return;
                    _isRefreshing = false;
                    Cursor = Cursors.Default;
                    SetBusy(false, "");
                    if (t.Exception != null)
                    {
                        MessageBox.Show(t.Exception.GetBaseException().Message, "Refresh failed", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        return;
                    }
                    _items = t.Result;
                    RenderList();
                }));
            });
        }

        private void SetBusy(bool busy, string message)
        {
            _refresh.Enabled = !busy;
            _refresh.Text = busy ? "Loading..." : "Refresh inventory";
            if (!string.IsNullOrWhiteSpace(message)) _hint.Text = message;
        }

        private void RenderList()
        {
            string q = (_search.Text ?? "").Trim().ToLowerInvariant();
            var rows = _items.Where(x => MatchesFilter(x) && (string.IsNullOrEmpty(q) || (x.HumanName() + " " + x.Name + " " + x.Command + " " + x.Source + " " + x.Location + " " + x.Status + " " + x.AdviceReason()).ToLowerInvariant().Contains(q))).ToList();
            _list.BeginUpdate(); _list.Items.Clear(); foreach (var x in rows) { var li = new ListViewItem(x.Enabled ? "Enabled" : "Disabled") { Tag = x }; li.SubItems.Add(x.HumanName()); li.SubItems.Add(x.Name); li.SubItems.Add(x.Source); li.SubItems.Add(x.RiskLabel()); li.SubItems.Add(x.AdviceLabel()); li.SubItems.Add(x.PopupLabel()); li.SubItems.Add(x.Location); li.SubItems.Add(x.Command); _list.Items.Add(li); } _list.EndUpdate();
            int highRisk = _items.Count(x => x.RiskLevel() == "Critical");
            int cleanup = _items.Count(x => x.AdviceLevel() == "Cleanup");
            _summary.Text = rows.Count + " visible / " + _items.Count + " total • " + _items.Count(x => x.Enabled) + " enabled • " + _items.Count(x => !x.Enabled) + " disabled • " + highRisk + " high risk • " + cleanup + " suggested cleanup";
            _visibleValue.Text = rows.Count.ToString(); _enabledValue.Text = _items.Count(x => x.Enabled).ToString(); _disabledValue.Text = _items.Count(x => !x.Enabled).ToString(); _reviewValue.Text = cleanup.ToString(); _managedValue.Text = _items.Count(x => x.IsManaged).ToString();
            _hint.Text = rows.Count == 0 ? "No startup items match this view. Clear search or switch filter." : "Application names are resolved from the executable metadata when possible. Red means high consequence; green REMOVE? means a conservative optional-startup cleanup candidate.";
            UpdateButtons();
        }

        private bool MatchesFilter(StartupItem item)
        {
            if (_filterMode == "Risky") return item.RiskLevel() == "Critical";
            if (_filterMode == "Cleanup") return item.AdviceLevel() == "Cleanup";
            if (_filterMode == "Disabled") return !item.Enabled || item.PopupLabel() == "Disabled";
            return true;
        }

        private void SetFilter(string mode)
        {
            _filterMode = mode;
            _showAll.BackColor = mode == "All" ? Accent : Surface2;
            _showRisky.BackColor = mode == "Risky" ? Danger : Surface2;
            _showCleanup.BackColor = mode == "Cleanup" ? Good : Surface2;
            _showDisabled.BackColor = mode == "Disabled" ? Accent : Surface2;
            RenderList();
        }

        private void MainFormKeyDown(object sender, KeyEventArgs e)
        {
            bool editingText = ActiveControl is TextBox;
            if (editingText && (e.KeyCode == Keys.Delete || e.KeyCode == Keys.Enter || (e.Control && e.KeyCode == Keys.C))) return;
            if (e.Control && e.KeyCode == Keys.N) { AddBootApp(); e.Handled = true; return; }
            if (e.KeyCode == Keys.F5) { RefreshItems(); e.Handled = true; return; }
            if (e.KeyCode == Keys.Delete) { DisableSelected(); e.Handled = true; return; }
            if (e.KeyCode == Keys.Enter) { EditSelected(); e.Handled = true; return; }
            if (e.Control && e.KeyCode == Keys.L) { LaunchSelectedNow(); e.Handled = true; return; }
            if (e.Control && e.KeyCode == Keys.O) { OpenSelectedLocation(); e.Handled = true; return; }
            if (e.Control && e.KeyCode == Keys.C) { CopySelectedCommand(); e.Handled = true; return; }
            if (e.KeyCode == Keys.Escape && !string.IsNullOrWhiteSpace(_search.Text)) { _search.Text = ""; e.Handled = true; return; }
        }

        private void SelectListItemOnRightClick(object sender, MouseEventArgs e)
        {
            if (e.Button != MouseButtons.Right) return;
            var hit = _list.HitTest(e.Location);
            if (hit.Item == null) return;
            _list.SelectedItems.Clear();
            hit.Item.Selected = true;
            hit.Item.Focused = true;
        }

        private void ListMouseUpPopupToggle(object sender, MouseEventArgs e)
        {
            var hit = _list.HitTest(e.Location);
            if (hit.Item == null || hit.SubItem == null) return;
            int col = hit.Item.SubItems.IndexOf(hit.SubItem);
            if (col != 6) return;
            if (!PopupButtonRect(hit.SubItem.Bounds).Contains(e.Location)) return;
            ToggleItemPopupState((StartupItem)hit.Item.Tag);
        }

        private void ToggleItemPopupState(StartupItem item)
        {
            if (item.PopupLabel() == "N/A") { _hint.Text = "Popup mode is not applicable to " + item.Source + " rows."; return; }
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

        private void UpdateButtons() { var x = Selected(); bool any = x != null; _editSelected.Enabled = any && x.PopupLabel() != "N/A"; _quietSelected.Enabled = any && x.PopupLabel() != "N/A" && x.PopupLabel() != "Disabled"; _disable.Enabled = any && x.Enabled && x.CanDisable; _enable.Enabled = any && !x.Enabled; _deleteManaged.Enabled = any && x.IsManaged && x.Source == "Scheduled Task"; }
        private StartupItem Selected() { return _list.SelectedItems.Count == 0 ? null : (StartupItem)_list.SelectedItems[0].Tag; }
        private void DisableSelected() { var x = Selected(); if (x == null) return; if (MessageBox.Show("Remove '" + x.Name + "' from Windows startup?", "Confirm remove", MessageBoxButtons.YesNo, MessageBoxIcon.Question) != DialogResult.Yes) return; try { StartupService.Disable(x); Toast("Removed from startup", x.Name + " will not run next boot."); RefreshItems(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Remove failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); } }
        private void EnableSelected() { var x = Selected(); if (x == null) return; try { StartupService.Enable(x); Toast("Restored", x.Name + " will run next boot."); RefreshItems(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Restore failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); } }
        private void DeleteManaged() { var x = Selected(); if (x == null || !x.IsManaged || x.Source != "Scheduled Task") { MessageBox.Show("Select a MichStartupMaster managed scheduled task."); return; } if (MessageBox.Show("Delete managed startup task '" + x.Name + "'?", "Confirm delete", MessageBoxButtons.YesNo, MessageBoxIcon.Warning) != DialogResult.Yes) return; try { StartupService.DeleteManagedTask(x.Location); RefreshItems(); Toast("Deleted", x.Name); } catch (Exception ex) { MessageBox.Show(ex.Message, "Delete failed"); } }
        private void MakeSelectedQuiet() { var x = Selected(); if (x == null) return; try { StartupService.SetPopupMode(x, false); Toast("Quiet protected", x.Name + " will start through the tray wrapper and be re-enforced."); RefreshItems(); } catch (Exception ex) { MessageBox.Show(ex.Message, "Quiet mode failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); } }
        private void EditSelected()
        {
            var x = Selected();
            if (x == null) return;
            try
            {
                string target, arguments;
                StartupService.ResolveLaunchTarget(x, out target, out arguments);
                using (var d = new AddStartupForm(x.HumanName(), target, arguments, x.PopupLabel() != "Enabled"))
                {
                    if (d.ShowDialog(this) != DialogResult.OK) return;
                    string task = StartupService.EditStartup(x, d.AppTitle, d.AppPath, d.AppArguments, d.TrayMode);
                    Toast("Edited startup", task);
                    RefreshItems();
                }
            }
            catch (Exception ex) { MessageBox.Show(ex.Message, "Edit failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); }
        }
        private void ProtectDisabledNow() { try { string result = ProtectedDisabledService.ProtectCurrentDisabled(); Toast("Protected disabled", result); } catch (Exception ex) { MessageBox.Show(ex.Message, "Protect disabled failed"); } }
        private void RunGuardsAsync(bool showResult)
        {
            Task.Run(() => ProtectedDisabledService.EnforceProtected() + " | " + ProtectedQuietService.EnforceProtected()).ContinueWith(t =>
            {
                if (IsDisposed || !showResult) return;
                BeginInvoke(new Action(() => Toast("Guards enforced", t.Exception == null ? t.Result : t.Exception.GetBaseException().Message)));
            });
        }
        private void OpenStartupFolders()
        {
            try
            {
                Process.Start("explorer.exe", Environment.GetFolderPath(Environment.SpecialFolder.Startup));
                Process.Start("explorer.exe", Environment.GetFolderPath(Environment.SpecialFolder.CommonStartup));
            }
            catch (Exception ex) { MessageBox.Show(ex.Message, "Open startup folders failed"); }
        }
        private void LaunchSelectedNow()
        {
            var x = Selected();
            if (x == null) return;
            try
            {
                string target, arguments, execute, actionArgs;
                StartupService.ResolveLaunchTarget(x, out target, out arguments);
                StartupService.BuildDirectAction(target, arguments, out execute, out actionArgs);
                var psi = new ProcessStartInfo(execute, actionArgs) { UseShellExecute = false, WorkingDirectory = Directory.Exists(Path.GetDirectoryName(target)) ? Path.GetDirectoryName(target) : Environment.GetFolderPath(Environment.SpecialFolder.UserProfile) };
                Process.Start(psi);
                Toast("Launched", x.HumanName());
            }
            catch (Exception ex) { MessageBox.Show(ex.Message, "Launch failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); }
        }
        private void OpenSelectedLocation()
        {
            var x = Selected();
            if (x == null) return;
            try
            {
                string target, arguments;
                StartupService.ResolveLaunchTarget(x, out target, out arguments);
                if (File.Exists(target)) Process.Start("explorer.exe", "/select," + Q(target));
                else if (Directory.Exists(x.Location)) Process.Start("explorer.exe", x.Location);
                else Clipboard.SetText(x.Location ?? "");
            }
            catch (Exception ex) { MessageBox.Show(ex.Message, "Open location failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); }
        }
        private void CopySelectedCommand()
        {
            var x = Selected();
            if (x == null) return;
            try
            {
                Clipboard.SetText(x.Command ?? "");
                Toast("Copied", "launch command");
            }
            catch (Exception ex) { MessageBox.Show(ex.Message, "Copy failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); }
        }
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
        private static string Q(string s) { return "\"" + (s ?? "").Replace("\"", "\\\"") + "\""; }
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

        public AddStartupForm() : this("", "", "", true) { }

        public AddStartupForm(string appTitle, string appPath, string appArguments, bool trayMode)
        {
            Text = "Add app to Windows startup"; Width = 720; Height = 520; FormBorderStyle = FormBorderStyle.FixedDialog; MaximizeBox = false; MinimizeBox = false; BackColor = Bg; ForeColor = Color.White; Font = new Font("Segoe UI", 10f); StartPosition = FormStartPosition.CenterParent; Icon = Program.AppIcon;
            Controls.Add(new Label { Text = string.IsNullOrWhiteSpace(appPath) ? "Add an app to startup" : "Edit startup app", Left = 28, Top = 24, AutoSize = true, ForeColor = TextMain, Font = new Font("Segoe UI Semibold", 22f) });
            Controls.Add(new Label { Text = "Choose an executable, script, shortcut, or command launcher, then pick normal startup or quiet tray startup.", Left = 30, Top = 66, Width = 620, Height = 40, ForeColor = Muted, Font = new Font("Segoe UI", 10.5f) });
            AddLabel("Friendly name", 118); _name = Box(144, 470);
            AddLabel("Executable path", 184); _path = Box(210, 486); var browse = Button("Browse", Accent, 82); browse.Left = 526; browse.Top = 208; browse.Click += Browse; Controls.Add(browse); var paste = Button("Paste", Surface, 76); paste.Left = 614; paste.Top = 208; paste.Click += PastePath; Controls.Add(paste);
            AddLabel("Optional arguments", 250); _args = Box(276, 622);
            AddLabel("Startup mode", 318);
            _normalMode = new RadioButton { Text = "Start normally - run the app directly at Windows logon", Left = 34, Top = 346, Width = 610, ForeColor = TextMain, BackColor = Bg, Checked = !trayMode };
            _trayMode = new RadioButton { Text = "Start quietly in tray mode - no terminal, minimized launch, controller tray icon", Left = 34, Top = 378, Width = 630, ForeColor = TextMain, BackColor = Bg, Checked = trayMode };
            Controls.Add(_normalMode); Controls.Add(_trayMode);
            Controls.Add(new Label { Text = "Quiet tray mode is the safest generic no-popup startup path; apps that force their own window may still show it.", Left = 54, Top = 406, Width = 590, Height = 34, ForeColor = Muted, Font = new Font("Segoe UI", 9f) });
            _name.Text = appTitle ?? "";
            _path.Text = appPath ?? "";
            _args.Text = appArguments ?? "";
            var ok = Button(string.IsNullOrWhiteSpace(appPath) ? "Add at next boot" : "Save startup", Accent, 150); ok.Left = 390; ok.Top = 452; ok.DialogResult = DialogResult.OK; ok.Click += ValidateBeforeClose;
            var cancel = Button("Cancel", Surface, 100); cancel.Left = 550; cancel.Top = 452; cancel.DialogResult = DialogResult.Cancel;
            Controls.Add(ok); Controls.Add(cancel); AcceptButton = ok; CancelButton = cancel;
        }
        private void ValidateBeforeClose(object sender, EventArgs e)
        {
            if (string.IsNullOrWhiteSpace(AppTitle) || string.IsNullOrWhiteSpace(AppPath) || !File.Exists(AppPath) || !StartupService.IsSupportedStartupTarget(AppPath))
            {
                MessageBox.Show("Choose a valid .exe, .cmd, .bat, .ps1, or .lnk file and a friendly name before saving startup.", "Missing app", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                DialogResult = DialogResult.None;
            }
        }
        private void AddLabel(string text, int top) { Controls.Add(new Label { Text = text, Left = 30, Top = top, AutoSize = true, ForeColor = Muted, Font = new Font("Segoe UI Semibold", 9.5f) }); }
        private TextBox Box(int top, int width) { var t = new TextBox { Left = 30, Top = top, Width = width, Height = 30, BackColor = Color.FromArgb(17, 24, 44), ForeColor = Color.White, BorderStyle = BorderStyle.FixedSingle, Font = new Font("Segoe UI", 10.5f) }; Controls.Add(t); return t; }
        private Button Button(string text, Color color, int width) { var b = new Button { Text = text, Width = width, Height = 38, FlatStyle = FlatStyle.Flat, BackColor = color, ForeColor = Color.White, Font = new Font("Segoe UI Semibold", 9.5f), Cursor = Cursors.Hand }; b.FlatAppearance.BorderSize = 0; return b; }
        private void Browse(object sender, EventArgs e) { using (var ofd = new System.Windows.Forms.OpenFileDialog { Filter = "Startup targets (*.exe;*.cmd;*.bat;*.ps1;*.lnk)|*.exe;*.cmd;*.bat;*.ps1;*.lnk|All files (*.*)|*.*", Title = "Choose app to start with Windows" }) if (ofd.ShowDialog(this) == DialogResult.OK) { _path.Text = ofd.FileName; if (string.IsNullOrWhiteSpace(_name.Text)) _name.Text = Path.GetFileNameWithoutExtension(ofd.FileName); } }
        private void PastePath(object sender, EventArgs e)
        {
            try
            {
                if (!Clipboard.ContainsText()) return;
                string text = (Clipboard.GetText() ?? "").Trim().Trim('"');
                if (File.Exists(text))
                {
                    _path.Text = text;
                    if (string.IsNullOrWhiteSpace(_name.Text)) _name.Text = Path.GetFileNameWithoutExtension(text);
                }
                else MessageBox.Show("Clipboard does not contain an existing startup target path.", "Paste path", MessageBoxButtons.OK, MessageBoxIcon.Information);
            }
            catch (Exception ex) { MessageBox.Show(ex.Message, "Paste path failed", MessageBoxButtons.OK, MessageBoxIcon.Warning); }
        }
    }

}
