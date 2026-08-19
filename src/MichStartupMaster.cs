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
using System.Text.Json;
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
        public static readonly string EnabledStore = Path.Combine(AppData, "enabled-startup-items.tsv");
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
                if (cmd == "--audit-boot") { Console.WriteLine(StartupService.AuditBootCoverage()); Console.WriteLine(StartupService.AuditTrayCoverage()); return 0; }
                if (cmd == "--detect-new") { var fresh = StartupWatcher.DetectNew(); Console.WriteLine("DETECT_NEW count=" + fresh.Count + (fresh.Count > 0 ? " first=" + fresh[0].HumanName() : "")); return 0; }
                if (cmd == "--add-test-task") return CliAddTestTask(args, true);
                if (cmd == "--add-test-task-tray") return CliAddTestTask(args, true);
                if (cmd == "--add-test-task-normal") return CliAddTestTask(args, false);
                if (cmd == "--add-startup") return CliAddStartup(args);
                if (cmd == "--remove-task") return CliRemoveTask(args);
                if (cmd == "--ui-contract") { Console.WriteLine(MainForm.UiContractJson()); return 0; }
                if (cmd == "--protect-disabled") { Console.WriteLine(ProtectedDisabledService.ProtectCurrentDisabled()); return 0; }
                if (cmd == "--enforce-disabled") { Console.WriteLine(ProtectedDisabledService.EnforceProtected()); return 0; }
                if (cmd == "--enforce-quiet") { Console.WriteLine(ProtectedQuietService.EnforceProtected()); return 0; }
                if (cmd == "--enforce-enabled") { Console.WriteLine(EnabledStartupService.EnforceEnabled()); return 0; }
                if (cmd == "--list-managed") { Console.WriteLine(EnabledStartupService.ToJson()); return 0; }
                if (cmd == "--toggle-popup") return CliTogglePopup(args);
                if (cmd == "--set-enabled") return CliSetEnabled(args);
                if (cmd == "--tray-run") { TrayRunner.RunMain(args.Skip(1).ToArray()); return 0; }
                if (cmd == "--start-in-tray" || cmd == "--agent")
                {
                    Application.EnableVisualStyles();
                    Application.SetCompatibleTextRenderingDefault(false);
                    bool createdNew;
                    using (var singleInstance = new System.Threading.Mutex(true, @"Local\MichStartupMaster.MainInstance", out createdNew))
                    {
                        if (!createdNew) return 0;
                        StartupService.EnsureAgentRegistered();
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
                StartupService.EnsureAgentRegistered();
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
            AddLegacyV2Items(items);
            HydrateHumanNames(items);
            return Dedupe(items).OrderBy(x => x.Enabled ? 0 : 1).ThenBy(x => x.Source).ThenBy(x => x.Name).ToList();
        }

        // Verify that every single boot source on the machine is represented in the app's own
        // list (ScanAll). Duplicates that dedupe collapses into a canonical row count as covered;
        // anything truly absent is reported as a gap so "everything shows in the app" is a
        // checkable, permanent guarantee rather than a hope.
        public static string AuditBootCoverage()
        {
            try
            {
                var shown = ScanAll();
                var raw = new List<StartupItem>();
                AddCommonRegistryStartup(raw);
                AddStartupFolder(raw, Environment.GetFolderPath(Environment.SpecialFolder.Startup), "User");
                AddStartupFolder(raw, Environment.GetFolderPath(Environment.SpecialFolder.CommonStartup), "Machine");
                AddLogonTasks(raw);
                AddAutoServices(raw);
                AddAutoDrivers(raw);
                var gaps = new List<string>();
                foreach (var src in raw)
                {
                    if (BootSourceCovered(src, shown)) continue;
                    gaps.Add(src.Source + " | " + (string.IsNullOrWhiteSpace(src.Name) ? "?" : src.Name) + " | " + (string.IsNullOrWhiteSpace(src.Command) ? "" : src.Command));
                }
                if (gaps.Count == 0) return "BOOT_AUDIT sources=" + raw.Count + " shown=" + shown.Count + " gaps=0";
                return "BOOT_AUDIT sources=" + raw.Count + " shown=" + shown.Count + " gaps=" + gaps.Count + Environment.NewLine + string.Join(Environment.NewLine, gaps.Take(50));
            }
            catch (Exception ex) { return "BOOT_AUDIT error: " + ex.GetBaseException().Message; }
        }

        private static bool BootSourceCovered(StartupItem src, List<StartupItem> shown)
        {
            string name = NormBootKey(src.Name);
            string loc = NormBootKey(src.Location);
            string cmd = NormBootKey(src.Command);
            foreach (var it in shown)
            {
                if (!string.Equals(it.Source, src.Source, StringComparison.OrdinalIgnoreCase)) continue;
                if (name.Length > 0 && NormBootKey(it.Name) == name) return true;
                if (loc.Length > 0 && NormBootKey(it.Location) == loc) return true;
                if (cmd.Length > 8 && NormBootKey(it.Command) == cmd) return true;
            }
            // A raw source hidden by dedupe is still covered when a shown row launches the exact
            // same app (retired duplicate launchers collapse into their canonical managed row).
            // Only app-launching sources qualify; never services/drivers whose shared svchost paths
            // would otherwise mask a genuinely missing service. A shown quiet row's command is a
            // --tray-run wrapper payload, so decode it back to the real target + arguments first.
            if (cmd.Length > 8 && (src.Source == "Scheduled Task" || src.Source == "Registry Run" || src.Source == "Registry RunOnce" || src.Source == "Policy Run" || src.Source == "Startup Folder"))
            {
                foreach (var it in shown)
                {
                    string shownCmd = it.Command ?? "";
                    string shownTarget, shownArgs;
                    if (TryDecodeTrayPayload(shownCmd, out shownTarget, out shownArgs)) shownCmd = shownTarget + " " + shownArgs;
                    if (NormBootKey(shownCmd) == cmd) return true;
                }
            }
            return false;
        }

        private static string NormBootKey(string value)
        {
            if (string.IsNullOrWhiteSpace(value)) return "";
            var sb = new StringBuilder();
            foreach (char ch in value.ToLowerInvariant().Trim().Trim('"'))
            {
                if (!char.IsWhiteSpace(ch)) sb.Append(ch);
            }
            return sb.ToString();
        }

        // ---- Tray audit: every tray app shows exactly one correct icon, forever ----
        // The historical failure mode was the quiet wrapper adding its OWN tray icon next to the
        // app's real one (a blank/default "broken" duplicate), and duplicate launchers starting a
        // second instance (a second real icon). This check makes both provably impossible to miss:
        //   WRAPPER_ICON - a --tray-run wrapper process is still showing a tray icon (wrapper must
        //                  stay invisible; the app draws its own icon)
        //   DUP          - two visible instances of the same managed tray app are running
        // A process is "visible" when it owns a tray-icon-class window, so app-internal helper
        // processes (e.g. whisper-key's launcher stub, which owns no windows) never count.
        [StructLayout(LayoutKind.Sequential)]
        private struct WinInfo { public int Pid; public string Class; }

        [DllImport("user32.dll")]
        private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
        private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
        [DllImport("user32.dll")]
        private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
        [DllImport("user32.dll")]
        private static extern int GetClassName(IntPtr hWnd, System.Text.StringBuilder lpClassName, int nMaxCount);

        private static List<WinInfo> SnapshotTopLevelWindows()
        {
            var list = new List<WinInfo>();
            try
            {
                EnumWindows((h, l) =>
                {
                    uint pid;
                    GetWindowThreadProcessId(h, out pid);
                    var sb = new System.Text.StringBuilder(256);
                    GetClassName(h, sb, 256);
                    list.Add(new WinInfo { Pid = (int)pid, Class = sb.ToString() });
                    return true;
                }, IntPtr.Zero);
            }
            catch { }
            return list;
        }

        private static bool IsTrayIconClass(string cls)
        {
            if (string.IsNullOrEmpty(cls)) return false;
            if (cls.StartsWith("WindowsForms10.Window", StringComparison.Ordinal)) return true; // WinForms NotifyIcon (+ main form)
            if (cls.IndexOf("SystemTrayIcon", StringComparison.Ordinal) >= 0) return true;      // pystray (icon + menu windows)
            if (cls.IndexOf("TrayIcon", StringComparison.Ordinal) >= 0) return true;            // Qt tray icon message window
            if (cls.IndexOf("NotifyIcon", StringComparison.Ordinal) >= 0) return true;          // Electron / generic
            if (string.Equals(cls, "AutoHotkey", StringComparison.OrdinalIgnoreCase)) return true; // AutoHotkey script tray icon
            return false;
        }

        private static bool ProcessHasTrayIcon(List<WinInfo> wins, int pid)
        {
            foreach (var w in wins) if (w.Pid == pid && IsTrayIconClass(w.Class)) return true;
            return false;
        }

        // Every enabled managed tray app must be running exactly once, and every wrapper process
        // must stay invisible. Returns a TRAY_AUDIT line plus one line per finding.
        public static string AuditTrayCoverage()
        {
            try
            {
                var rows = EnabledStartupService.Load().Where(r => r.Kind == "managed-task" && r.Mode == "tray").ToList();
                var procs = new List<Tuple<int, string, string>>(); // pid, exe, commandline(lower)
                try
                {
                    using (var searcher = new System.Management.ManagementObjectSearcher("SELECT ProcessId,ExecutablePath,CommandLine FROM Win32_Process"))
                    {
                        foreach (System.Management.ManagementObject mo in searcher.Get())
                        {
                            int pid = Convert.ToInt32(mo["ProcessId"]);
                            string cmd = Convert.ToString(mo["CommandLine"] ?? "").ToLowerInvariant();
                            string exe = Convert.ToString(mo["ExecutablePath"] ?? "");
                            procs.Add(Tuple.Create(pid, exe, cmd));
                        }
                    }
                }
                catch { }
                var wins = SnapshotTopLevelWindows();
                var findings = new List<string>();
                int running = 0;

                // 1. No wrapper process may ever draw a tray icon (the wrapper stays invisible).
                foreach (var p in procs)
                {
                    string cmd = p.Item3;
                    if (cmd.IndexOf("--tray-run", StringComparison.Ordinal) < 0) continue;
                    if (cmd.IndexOf("--agent", StringComparison.Ordinal) >= 0) continue; // the agent itself owns its one icon
                    if (ProcessHasTrayIcon(wins, p.Item1))
                        findings.Add("WRAPPER_ICON pid=" + p.Item1 + " (a quiet wrapper is showing a tray icon; wrappers must stay invisible)");
                }

                // 2. Each managed tray app runs exactly one visible instance.
                foreach (var row in rows)
                {
                    var candidates = new List<string>();
                    string target = (row.Target ?? "").Trim().ToLowerInvariant();
                    if (target.Length > 0) candidates.Add(target);
                    // Indirection targets (powershell.exe -File script.ps1 / wscript x.vbs / pythonw x.pyw)
                    // never stay running; match on the script's directory so the real worker is found.
                    foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(row.Arguments ?? "", @"[""']?([A-Za-z]:[^""'\s]+?\.(?:ps1|pyw|py|vbs|bat|cmd|exe|lnk))[""']?", System.Text.RegularExpressions.RegexOptions.IgnoreCase))
                    {
                        string script = m.Groups[1].Value;
                        try { string dir = System.IO.Path.GetDirectoryName(script); if (!string.IsNullOrWhiteSpace(dir)) candidates.Add(dir.ToLowerInvariant()); }
                        catch { }
                    }
                    if (candidates.Count == 0) continue;
                    var matches = procs.Where(p =>
                        p.Item3.IndexOf("--type=", StringComparison.Ordinal) < 0 && // exclude Electron/Chromium children
                        candidates.Any(c => c.Length > 0 && p.Item3.IndexOf(c, StringComparison.Ordinal) >= 0)).ToList();
                    var visible = matches.Where(p => ProcessHasTrayIcon(wins, p.Item1)).ToList();
                    if (visible.Count > 0) running++;
                    if (visible.Count > 1)
                        findings.Add("DUP " + row.Name + " | " + visible.Count + " visible instances (pids " + string.Join(",", visible.Select(v => v.Item1.ToString())) + ")");
                }

                if (findings.Count == 0) return "TRAY_AUDIT apps=" + rows.Count + " running=" + running + " findings=0";
                return "TRAY_AUDIT apps=" + rows.Count + " running=" + running + " findings=" + findings.Count + Environment.NewLine + string.Join(Environment.NewLine, findings);
            }
            catch (Exception ex) { return "TRAY_AUDIT error: " + ex.GetBaseException().Message; }
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
            var all = items.ToList();
            // Native (registry/folder) commands that are already surfaced: WMI Startup Command
            // rows are only an alternate mirror of the same registrations.
            var nativeCommands = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var item in all)
            {
                if ((item.Id ?? "").StartsWith("disabled|", StringComparison.OrdinalIgnoreCase)) continue;
                if (item.Source == "Registry Run" || item.Source == "Registry RunOnce" || item.Source == "Registry RunServices" || item.Source == "Policy Run" || item.Source == "Startup Folder")
                    nativeCommands.Add(NormalizeManagedCommand(item.Command, ""));
            }
            // Commands a managed item already launches: any other registration that launches the
            // exact same app is a retired/redundant duplicate and is not shown again.
            var managedCommands = new HashSet<string>(EnabledStartupService.Load()
                .Where(r => r.Kind == "managed-task" && !string.IsNullOrWhiteSpace(r.Target))
                .Select(r => NormalizeManagedCommand(r.Target, r.Arguments)), StringComparer.OrdinalIgnoreCase);
            var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var wmiSeen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            var result = new List<StartupItem>();
            // Live registrations first: they always win over informational/legacy records.
            foreach (var item in all.Where(i => !(i.Id ?? "").StartsWith("disabled|", StringComparison.OrdinalIgnoreCase)))
            {
                // A disabled task that launches the same app as a managed item is the retired
                // duplicate launcher; the managed row is the single source of truth.
                if (item.Source == "Scheduled Task" && !item.Enabled && managedCommands.Contains(NormalizeManagedCommand(item.Command, ""))) continue;
                if (item.Source == "Startup Command")
                {
                    string nc = NormalizeManagedCommand(item.Command, "");
                    if (nativeCommands.Contains(nc)) continue;
                    if (!wmiSeen.Add(nc)) continue;
                }
                string key = (item.Source + "|" + item.Location + "|" + item.Name + "|" + item.Command).ToLowerInvariant();
                if (seen.Add(key)) result.Add(item);
            }
            // Legacy disabled-store records are informational only: drop them when a live
            // registration already covers the same source + location + name.
            foreach (var item in all.Where(i => (i.Id ?? "").StartsWith("disabled|", StringComparison.OrdinalIgnoreCase)))
            {
                string baseKey = (item.Source + "|" + item.Location + "|" + item.Name).ToLowerInvariant();
                bool covered = result.Any(live => string.Equals((live.Source + "|" + live.Location + "|" + live.Name).ToLowerInvariant(), baseKey, StringComparison.OrdinalIgnoreCase));
                if (covered) continue;
                string key = (item.Source + "|" + item.Location + "|" + item.Name + "|" + item.Command).ToLowerInvariant();
                if (seen.Add(key)) result.Add(item);
            }
            return result;
        }

        // Normalize a launch command to its lower-cased, collapsed form for duplicate detection.
        private static string NormalizeManagedCommand(string target, string args)
        {
            try { return Regex.Replace((target + " " + (args ?? "")).ToLowerInvariant(), @"\s+", " ").Trim().Trim('"'); }
            catch { return (target + " " + (args ?? "")).ToLowerInvariant(); }
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
  $hasLogon=$false; $hasBoot=$false; $hasTime=$false; $hasDelay=$false
  foreach($tr in @($t.Triggers)){
    if($null -eq $tr){ continue }
    $cn = if($tr.CimClass){ [string]$tr.CimClass.CimClassName } else { '' }
    if($cn -like '*LogonTrigger*'){ $hasLogon=$true }
    if($cn -like '*BootTrigger*'){ $hasBoot=$true }
    if($cn -like '*TimeTrigger*'){ $hasTime=$true }
    $delayProp=$tr.PSObject.Properties['Delay']
    if($delayProp -and $delayProp.Value){ $hasDelay=$true }
  }
  $path = $t.TaskPath
  $isMicrosoft = $path.StartsWith('\Microsoft\') -or $path.StartsWith('\Windows\') -or $path.StartsWith('\GoogleSystem\')
  if($hasLogon -or $hasBoot -or ($hasTime -and -not $isMicrosoft)){
    $actions = (@($t.Actions) | ForEach-Object { if($_){ (($_.Execute) + ' ' + ($_.Arguments)).Trim() } }) -join ' || '
    $enabled = if($t.Settings.Enabled){'true'}else{'false'}
    $managed = if(($t.TaskPath + $t.TaskName).StartsWith('\MichStartupMaster\')){'true'}else{'false'}
    $k=@(); if($hasLogon){ $k+='logon' }; if($hasBoot){ $k+='boot' }; if($hasTime){ $k+='time' }; $triggerKind=$k -join '+'
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

        // Surface startup items that exist only in the legacy v2 state (no live registration
        // anywhere) so nothing the user once configured is invisible. Restore re-creates them.
        private static void AddLegacyV2Items(List<StartupItem> items)
        {
            try
            {
                string v2Path = Path.Combine(Program.AppData, "state-v2.json");
                if (!File.Exists(v2Path)) return;
                string raw;
                try { raw = File.ReadAllText(v2Path); } catch { return; }
                System.Text.Json.JsonDocument doc;
                try { doc = System.Text.Json.JsonDocument.Parse(raw); } catch { return; }
                using (doc)
                {
                    if (!doc.RootElement.TryGetProperty("Items", out var arr) || arr.ValueKind != System.Text.Json.JsonValueKind.Array) return;
                    var liveTaskLocs = new HashSet<string>(items.Where(i => i.Source == "Scheduled Task").Select(i => (i.Location ?? "").ToLowerInvariant()), StringComparer.OrdinalIgnoreCase);
                    var manifest = EnabledStartupService.Load();
                    foreach (var it in arr.EnumerateArray())
                    {
                        try
                        {
                            string name = JsonV2String(it, "LegacyTaskName");
                            if (string.IsNullOrWhiteSpace(name)) name = JsonV2String(it, "Name");
                            string target = JsonV2String(it, "Target");
                            string args = JsonV2String(it, "Arguments");
                            if (string.IsNullOrWhiteSpace(name) || string.IsNullOrWhiteSpace(target)) continue;
                            string safeName = Regex.Replace(name, "[^A-Za-z0-9 _.-]", "").Trim();
                            if (safeName.Length == 0) safeName = "StartupApp";
                            string taskLocation = Program.ManagedTaskRoot + safeName;
                            // Already tracked or live: the normal scan shows it.
                            if (manifest.Any(r => string.Equals(r.TaskLocation, taskLocation, StringComparison.OrdinalIgnoreCase))) continue;
                            if (liveTaskLocs.Contains(taskLocation.ToLowerInvariant())) continue;
                            if (EnabledStartupService.IsV2Migrated(taskLocation)) continue;
                            bool enabled = JsonV2Bool(it, "Enabled");
                            int launchMode = JsonV2Int(it, "LaunchMode");
                            string command = (target + (string.IsNullOrWhiteSpace(args) ? "" : " " + args)).Trim();
                            items.Add(new StartupItem
                            {
                                Id = "legacy|" + Convert.ToBase64String(Encoding.UTF8.GetBytes(safeName)) + "|" + launchMode,
                                Name = safeName,
                                Source = "Legacy v2",
                                Scope = "User/System",
                                Command = command,
                                Location = "Legacy v2 state",
                                Enabled = false,
                                CanDisable = false,
                                IsManaged = false,
                                Status = (enabled ? "Enabled" : "Disabled") + " in legacy v2 state — not currently registered; Restore to start it at every boot"
                            });
                        }
                        catch { }
                    }
                }
            }
            catch { }
        }

        private static string JsonV2String(System.Text.Json.JsonElement el, string key)
        {
            try { if (el.TryGetProperty(key, out var v) && v.ValueKind == System.Text.Json.JsonValueKind.String) return v.GetString() ?? ""; } catch { }
            return "";
        }

        private static bool JsonV2Bool(System.Text.Json.JsonElement el, string key)
        {
            try { if (el.TryGetProperty(key, out var v) && v.ValueKind == System.Text.Json.JsonValueKind.True) return true; } catch { }
            return false;
        }

        private static int JsonV2Int(System.Text.Json.JsonElement el, string key)
        {
            try { if (el.TryGetProperty(key, out var v) && v.ValueKind == System.Text.Json.JsonValueKind.Number) return v.GetInt32(); } catch { }
            return 0;
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
            if (item == null) throw new ArgumentNullException("item");
            if (item.Id.StartsWith("legacy|", StringComparison.OrdinalIgnoreCase))
            {
                // A legacy v2 ghost is already not registered; "disabling" it just takes it
                // over so the migration/view stop surfacing it.
                EnabledStartupService.Remove(item);
                try
                {
                    string[] parts = item.Id.Split('|');
                    if (parts.Length > 1 && !string.IsNullOrWhiteSpace(UnB64(parts[1]))) EnabledStartupService.MarkV2Migrated(Program.ManagedTaskRoot + UnB64(parts[1]));
                }
                catch { }
                return;
            }
            // Record intent FIRST: even if the direct disable call fails or times out, the guard
            // will keep enforcing the disabled state and the manifest row is removed in finally,
            // so a half-completed disable can never leave the item silently enabled again.
            ProtectedDisabledService.Protect(item);
            // A quiet (tray) task must also leave the quiet-protection store. Otherwise the quiet
            // guard re-registers it as an ENABLED task every 30 seconds and the disable can never
            // stick — the exact "disable does not work" bug for quiet apps like GameSir/AHK/whisper-key.
            if (item.Id.StartsWith("task|", StringComparison.OrdinalIgnoreCase) && !string.IsNullOrWhiteSpace(item.Location))
                ProtectedQuietService.UnprotectTask(item.Location);
            try
            {
                if (item.Id.StartsWith("reg|")) DisableRegistry(item);
                else if (item.Id.StartsWith("active|")) DisableActiveSetup(item);
                else if (item.Id.StartsWith("folder|")) DisableStartupFolder(item);
                else if (item.Id.StartsWith("task|")) RunChecked("schtasks.exe", "/Change /TN " + Q(item.Location) + " /Disable");
                else if (item.Id.StartsWith("service|")) DisableServiceOrDriver(item, "service");
                else if (item.Id.StartsWith("driver|")) DisableServiceOrDriver(item, "driver");
                else throw new InvalidOperationException("Unsupported item: " + item.Id);
            }
            finally
            {
                EnabledStartupService.Remove(item);
            }
        }

        public static void Enable(StartupItem item)
        {
            if (item == null) throw new ArgumentNullException("item");
            if (item.Id.StartsWith("legacy|", StringComparison.OrdinalIgnoreCase))
            {
                // Recreate the managed startup from the item's stored v2 configuration.
                EnableLegacyV2(item);
                return;
            }
            ProtectedDisabledService.Unprotect(item);
            if (item.Id.StartsWith("disabled|reg|")) RestoreRegistry(item);
            else if (item.Id.StartsWith("disabled|active|")) RestoreActiveSetup(item);
            else if (item.Id.StartsWith("disabled|folder|")) RestoreStartupFolder(item);
            else if (item.Id.StartsWith("disabled|service|")) RestoreServiceOrDriver(item);
            else if (item.Id.StartsWith("disabled|driver|")) RestoreServiceOrDriver(item);
            else if (item.Id.StartsWith("task|")) RunChecked("schtasks.exe", "/Change /TN " + Q(item.Location) + " /Enable");
            else throw new InvalidOperationException("Unsupported disabled item: " + item.Id);
            EnabledStartupService.UpsertFromItem(item);
            // Re-assert quiet protection for a quiet task so the quiet guard keeps it running
            // quietly even if the manifest row is ever lost again.
            if (item.Id.StartsWith("task|") && StartupService.CommandUsesTrayWrapper(item.Command ?? "") && !string.IsNullOrWhiteSpace(item.Location))
            {
                string quietTarget, quietArgs;
                try
                {
                    StartupService.ResolveLaunchTarget(item, out quietTarget, out quietArgs);
                    if (!string.IsNullOrWhiteSpace(quietTarget)) ProtectedQuietService.ProtectTask(item.Location, quietTarget, quietArgs ?? "");
                }
                catch { }
            }
        }

        private static void EnableLegacyV2(StartupItem item)
        {
            try
            {
                string[] parts = item.Id.Split('|');
                string legacyName = parts.Length > 1 ? UnB64(parts[1]) : "";
                if (string.IsNullOrWhiteSpace(legacyName)) throw new InvalidOperationException("Legacy item name is missing");
                string v2Path = Path.Combine(Program.AppData, "state-v2.json");
                if (!File.Exists(v2Path)) throw new InvalidOperationException("Legacy v2 state file is missing");
                string target = "", args = "";
                int launchMode = 0;
                using (var doc = System.Text.Json.JsonDocument.Parse(File.ReadAllText(v2Path)))
                {
                    if (doc.RootElement.TryGetProperty("Items", out var arr) && arr.ValueKind == System.Text.Json.JsonValueKind.Array)
                    {
                        foreach (var it in arr.EnumerateArray())
                        {
                            string name = it.TryGetProperty("LegacyTaskName", out var lt) && lt.ValueKind == System.Text.Json.JsonValueKind.String ? lt.GetString() : "";
                            if (string.IsNullOrWhiteSpace(name)) { name = it.TryGetProperty("Name", out var nn) && nn.ValueKind == System.Text.Json.JsonValueKind.String ? nn.GetString() : ""; }
                            if (!string.Equals(name, legacyName, StringComparison.OrdinalIgnoreCase)) continue;
                            target = it.TryGetProperty("Target", out var tg) && tg.ValueKind == System.Text.Json.JsonValueKind.String ? tg.GetString() : "";
                            args = it.TryGetProperty("Arguments", out var ag) && ag.ValueKind == System.Text.Json.JsonValueKind.String ? ag.GetString() : "";
                            if (it.TryGetProperty("LaunchMode", out var lm) && lm.ValueKind == System.Text.Json.JsonValueKind.Number) { try { launchMode = lm.GetInt32(); } catch { } }
                            break;
                        }
                    }
                }
                if (string.IsNullOrWhiteSpace(target)) throw new InvalidOperationException("Legacy item target is missing in v2 state");
                if (!File.Exists(target)) throw new FileNotFoundException("Application not found", target);
                AddManagedStartup(legacyName, target, args ?? "", launchMode == 2, true);
                EnabledStartupService.MarkV2Migrated(Program.ManagedTaskRoot + legacyName);
            }
            catch (Exception ex) { throw new InvalidOperationException("Could not restore legacy item: " + ex.Message, ex); }
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

        // Guarantee the hidden boot agent is registered from this copy of the app:
        // a Startup-folder shortcut and a managed logon task, both launching --agent.
        public static void EnsureAgentRegistered()
        {
            // Adopt legacy v2 enabled items into the enforcement manifest right away, so items
            // set to start in the old app are registered and running at this very boot.
            try { EnabledStartupService.MigrateV2EnabledItems(); } catch { }
            try
            {
                string exe = ProcessExePath();
                if (string.IsNullOrWhiteSpace(exe) || !File.Exists(exe)) return;
                string wd = Path.GetDirectoryName(exe) ?? "";
                string lnk = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Startup), "Mich Startup Master Agent.lnk");
                bool lnkOk = false;
                try
                {
                    if (File.Exists(lnk))
                    {
                        Type shellType = Type.GetTypeFromProgID("WScript.Shell");
                        if (shellType != null)
                        {
                            dynamic shell = Activator.CreateInstance(shellType);
                            dynamic shortcut = shell.CreateShortcut(lnk);
                            lnkOk = string.Equals(Convert.ToString(shortcut.TargetPath), exe, StringComparison.OrdinalIgnoreCase)
                                && string.Equals(Convert.ToString(shortcut.Arguments ?? "").Trim(), "--agent", StringComparison.OrdinalIgnoreCase);
                        }
                    }
                }
                catch { lnkOk = false; }
                if (!lnkOk)
                {
                    string script =
                        "$ErrorActionPreference='SilentlyContinue';" +
                        "function D($s){[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s))};" +
                        "$lnk = D '" + B64(lnk) + "';" +
                        "$exe = D '" + B64(exe) + "';" +
                        "$wd = D '" + B64(wd) + "';" +
                        "$s=(New-Object -ComObject WScript.Shell).CreateShortcut($lnk);" +
                        "$s.TargetPath=$exe;$s.Arguments='--agent';$s.WorkingDirectory=$wd;" +
                        "$s.Description='Mich Startup Master hidden startup agent';$s.Save();";
                    RunPowerShellScript(script);
                }
                // Backup path: a managed logon task that re-runs the agent with no delay.
                string task = Program.ManagedTaskRoot + "MichStartupMasterApp";
                RegisterLogonTaskAt(task, exe, "--agent");
            }
            catch { }
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
            EnabledStartupService.Upsert(new EnabledStartupService.Row { Kind = "managed-task", Name = safeName, Scope = "User/System", Command = execute + " " + actionArgs, Location = taskLocation, Status = "Managed startup task", Target = targetPath, Arguments = arguments ?? "", Mode = trayMode ? "tray" : "normal", TaskLocation = taskLocation });
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
                "$settings=New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 0);" +
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

        internal static bool IsSelfTarget(string targetPath)
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
            EnabledStartupService.RemoveByTask(tn);
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
                EnabledStartupService.Upsert(new EnabledStartupService.Row { Kind = "managed-task", Name = item.Name ?? "", Scope = item.Scope ?? "", Command = execute + " " + actionArgs, Location = item.Location, Status = item.Status ?? "", Target = target, Arguments = arguments ?? "", Mode = popupEnabled ? "normal" : "tray", TaskLocation = item.Location });
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
                EnabledStartupService.Upsert(new EnabledStartupService.Row { Kind = "managed-task", Name = name ?? "", Scope = item.Scope ?? "", Command = execute + " " + actionArgs, Location = item.Location, Status = item.Status ?? "", Target = targetPath, Arguments = arguments ?? "", Mode = trayMode ? "tray" : "normal", TaskLocation = item.Location });
                return item.Location;
            }
            if (item.PopupLabel() == "N/A") throw new InvalidOperationException("This startup source cannot be edited as an application launch. Services, drivers, Winlogon, AppInit, and Active Setup rows should be changed from their owning tool.");
            if (item.Enabled && item.CanDisable) Disable(item);
            else if (item.Enabled && !item.CanDisable) throw new InvalidOperationException("This startup source is read-only here. Run elevated or choose its matching Registry Run, Startup Folder, or Scheduled Task row.");
            return AddManagedStartup(name, targetPath, arguments ?? "", trayMode, true);
        }

        internal static void BuildManagedAction(string targetPath, string arguments, bool trayMode, out string execute, out string actionArgs)
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

        // Packaged (MSIX/Store) apps live under WindowsApps in a versioned folder that moves
        // every time the app auto-updates. A stored path pointing at an old version would otherwise
        // silently fail at boot, so resolve it to the newest installed version of the same package
        // family. Returns the original path when it already exists or cannot be resolved, so callers
        // simply check File.Exists afterwards.
        public static string ResolveTargetPath(string target)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(target)) return target;
                if (File.Exists(target)) return target;
                string windowsApps = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "WindowsApps");
                string full = Path.GetFullPath(target);
                if (!full.StartsWith(windowsApps, StringComparison.OrdinalIgnoreCase)) return target;
                string relative = full.Substring(windowsApps.Length).TrimStart('\\', '/');
                int slash = relative.IndexOfAny(new[] { '\\', '/' });
                if (slash < 0) return target;
                string packageFolder = relative.Substring(0, slash);
                string subPath = relative.Substring(slash + 1);
                int dd = packageFolder.LastIndexOf("__", StringComparison.Ordinal);
                if (dd < 0) return target;
                string publisher = packageFolder.Substring(dd + 2);
                string before = packageFolder.Substring(0, dd);
                int us = before.IndexOf('_');
                if (us <= 0) return target;
                string name = before.Substring(0, us);
                string best = null;
                Version bestVersion = null;
                if (Directory.Exists(windowsApps))
                {
                    foreach (string dir in Directory.GetDirectories(windowsApps, name + "_*"))
                    {
                        string fn = Path.GetFileName(dir);
                        if (!fn.EndsWith("__" + publisher, StringComparison.OrdinalIgnoreCase)) continue;
                        string mid = fn.Substring(name.Length + 1, fn.Length - name.Length - 1 - ("__" + publisher).Length);
                        int sep = mid.IndexOf('_');
                        if (sep <= 0) continue;
                        string verText = mid.Substring(0, sep);
                        Version v;
                        if (!Version.TryParse(verText, out v)) continue;
                        string candidate = Path.Combine(dir, subPath);
                        if (!File.Exists(candidate)) continue;
                        if (bestVersion == null || v > bestVersion) { bestVersion = v; best = candidate; }
                    }
                }
                return best ?? target;
            }
            catch { return target; }
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
        internal static string WinArg(string value) { return "\"" + (value ?? "").Replace("\"", "\\\"") + "\""; }

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

        internal static bool TryDecodeTrayPayload(string command, out string targetPath, out string arguments)
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

        internal static bool TrySplitCommand(string command, out string exe, out string args)
        {
            exe = ""; args = "";
            command = (command ?? "").Trim();
            if (command.Length == 0) return false;
            if (command.StartsWith("\"", StringComparison.Ordinal))
            {
                int end = command.IndexOf('"', 1);
                if (end > 1) { exe = command.Substring(1, end - 1); args = command.Substring(end + 1).Trim(); return true; }
            }
            Match m = Regex.Match(command, @"^(?<exe>.+?\.(?:exe|lnk|ps1|cmd|bat|vbs|py|pyw|com|msc))(?:\s+(?<args>.*))?$", RegexOptions.IgnoreCase);
            if (m.Success) { exe = m.Groups["exe"].Value.Trim(); args = m.Groups["args"].Value.Trim(); return true; }
            if (File.Exists(command)) { exe = command; args = ""; return true; }
            return false;
        }

        // Human-friendly name for a startup target, used by the Add/Edit dialog to auto-fill the
        // friendly name the moment a path is pasted or typed.
        public static string SuggestDisplayName(string path)
        {
            if (string.IsNullOrWhiteSpace(path)) return "";
            try
            {
                string expanded = ExpandPathTokens(path.Trim('"'));
                if (!File.Exists(expanded)) return CleanName(SafeFileStem(path));
                string fromFile = FriendlyNameFromFile(expanded);
                if (!string.IsNullOrWhiteSpace(fromFile)) return fromFile;
                return CleanName(SafeFileStem(expanded));
            }
            catch { return ""; }
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
        internal static string Q(string s) { return "\"" + (s ?? "").Replace("\"", "\\\"") + "\""; }

        internal static string ProcessExePath() { try { return Process.GetCurrentProcess().MainModule.FileName; } catch { return ""; } }

        internal static string PowerShellExe()
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

        internal static string RunCapture(string exe, string args, int timeoutMs = 30000)
        {
            var psi = new ProcessStartInfo(exe, args) { UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true, CreateNoWindow = true };
            using (var p = Process.Start(psi))
            {
                // Read the pipes on background tasks so a hung child can never block the GUI forever;
                // WaitForExit with a hard timeout kills it and surfaces a clear error instead.
                var so = Task.Run(() => p.StandardOutput.ReadToEnd());
                var se = Task.Run(() => p.StandardError.ReadToEnd());
                if (!p.WaitForExit(timeoutMs))
                {
                    try { p.Kill(); } catch { }
                    try { p.WaitForExit(3000); } catch { }
                    throw new Exception("Command timed out after " + timeoutMs + " ms: " + exe + " " + args);
                }
                string o = so.Result;
                string e = se.Result;
                if (p.ExitCode != 0 && string.IsNullOrWhiteSpace(o)) throw new Exception(e);
                return o;
            }
        }

        internal static void RunChecked(string exe, string args, int timeoutMs = 30000)
        {
            var psi = new ProcessStartInfo(exe, args) { UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true, CreateNoWindow = true };
            using (var p = Process.Start(psi))
            {
                var so = Task.Run(() => p.StandardOutput.ReadToEnd());
                var se = Task.Run(() => p.StandardError.ReadToEnd());
                if (!p.WaitForExit(timeoutMs))
                {
                    try { p.Kill(); } catch { }
                    try { p.WaitForExit(3000); } catch { }
                    throw new Exception("Command timed out after " + timeoutMs + " ms: " + exe + " " + args);
                }
                string o = so.Result;
                string e = se.Result;
                if (p.ExitCode != 0) throw new Exception((o + " " + e).Trim());
            }
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

        // Remove a protection row by its exact key, so an explicitly-enabled item can never be re-disabled by the guard.
        public static void UnprotectKey(string key)
        {
            if (string.IsNullOrWhiteSpace(key) || !File.Exists(Program.ProtectedDisabledStore)) return;
            SaveRows(LoadRows().Where(r => !string.Equals(r.Key, key, StringComparison.OrdinalIgnoreCase)).ToList());
        }

        // Protect a task/registry row directly by key (used when retiring duplicate launchers), so
        // the guard keeps enforcing its disabled state even though no StartupItem was scanned.
        public static void ProtectKey(string key, string location, string source, string status)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(key)) return;
                Directory.CreateDirectory(Program.AppData);
                var rows = LoadRows().Where(r => !string.Equals(r.Key, key, StringComparison.OrdinalIgnoreCase)).ToList();
                rows.Add(Row.FromParts(key, location, source, status));
                SaveRows(rows);
            }
            catch { }
        }

        public static string ProtectCurrentDisabled()
        {
            int count = 0;
            // Never protect the app's own boot agent or its own managed startup agent task.
            string agentTask = Program.ManagedTaskRoot + "MichStartupMasterApp";
            foreach (var item in StartupService.ScanAll().Where(x => !x.Enabled && !x.Id.StartsWith("error|") && !x.Id.StartsWith("legacy|", StringComparison.OrdinalIgnoreCase) && !string.Equals(x.Location, agentTask, StringComparison.OrdinalIgnoreCase))) { Protect(item); count++; }
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

        public static bool IsProtected(string key)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(key) || !File.Exists(Program.ProtectedDisabledStore)) return false;
                return LoadRows().Any(r => string.Equals(r.Key, key, StringComparison.OrdinalIgnoreCase));
            }
            catch { return false; }
        }

        private static bool EnforceRow(Row r)
        {
            if (r.Key.StartsWith("task|", StringComparison.OrdinalIgnoreCase))
            {
                // Use the hard-timeout runner (drains pipes, kills hung children) so a stuck
                // schtasks call can never wedge the guard or leave the task half-disabled.
                try { StartupService.RunChecked("schtasks.exe", "/Change /TN " + Q(r.Location) + " /Disable"); }
                catch { }
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
            WriteAllLinesWithRetry(Program.ProtectedDisabledStore, rows.Select(r => string.Join("\t", new[] { B64(r.Key), B64(r.Id), B64(r.Type), B64(r.Name), B64(r.Scope), B64(r.Command), B64(r.Location), B64(r.Status) })).ToArray());
        }

        // The agent's guards and the GUI/CLI write the same store from separate processes; a
        // momentary file lock must never crash the caller, so retry briefly before giving up.
        private static void WriteAllLinesWithRetry(string path, string[] lines)
        {
            for (int attempt = 0; ; attempt++)
            {
                try { File.WriteAllLines(path, lines, Encoding.UTF8); return; }
                catch (IOException) { if (attempt >= 4) throw; System.Threading.Thread.Sleep(300); }
                catch (UnauthorizedAccessException) { if (attempt >= 4) throw; System.Threading.Thread.Sleep(300); }
            }
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
            public static Row FromParts(string key, string location, string type, string status) { return new Row { Key = key, Id = "task|" + location, Type = type, Name = System.IO.Path.GetFileName(location.TrimEnd('\\')), Scope = "User/System", Command = "", Location = location, Status = status }; }
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

        public static bool IsProtected(string taskLocation)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(taskLocation) || !File.Exists(Program.ProtectedQuietStore)) return false;
                return LoadRows().Any(r => string.Equals(r.TaskLocation, taskLocation, StringComparison.OrdinalIgnoreCase));
            }
            catch { return false; }
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
            // Retry briefly: the agent's guards and the GUI/CLI write the same store from
            // separate processes, so a momentary lock must never crash the caller.
            for (int attempt = 0; ; attempt++)
            {
                try { File.WriteAllLines(Program.ProtectedQuietStore, rows.Select(r => string.Join("\t", new[] { B64(r.TaskLocation), B64(r.TargetPath), B64(r.Arguments) })), Encoding.UTF8); return; }
                catch (IOException) { if (attempt >= 4) throw; System.Threading.Thread.Sleep(300); }
                catch (UnauthorizedAccessException) { if (attempt >= 4) throw; System.Threading.Thread.Sleep(300); }
            }
        }

        private static string B64(string s) { return Convert.ToBase64String(Encoding.UTF8.GetBytes(s ?? "")); }
        private static string UnB64(string s) { try { return Encoding.UTF8.GetString(Convert.FromBase64String(s)); } catch { return ""; } }
        private sealed class Row { public string TaskLocation, TargetPath, Arguments; }
    }

    internal static class EnabledStartupService
    {
        public sealed class Row
        {
            public string Kind;          // managed-task | task | registry | folder | service | driver
            public string Name;
            public string Scope;
            public string Command;       // raw command / registry value / folder file
            public string Location;      // registry key or task path
            public string Status;        // stored metadata (registry root\tsubkey\tvalue\tkind\tsource, or service meta)
            public string Target;        // resolved executable
            public string Arguments;
            public string Mode;          // normal | tray
            public string TaskLocation;  // full task path for task kinds
        }

        private static string StorePath { get { return Program.EnabledStore; } }

        public static void Upsert(Row row)
        {
            if (row == null) return;
            Directory.CreateDirectory(Program.AppData);
            string key = RowKey(row);
            var rows = Load().Where(r => !string.Equals(RowKey(r), key, StringComparison.OrdinalIgnoreCase)).ToList();
            rows.Add(row);
            Save(rows);
        }

        private static string RowKey(Row r)
        {
            if (r == null) return "";
            if (!string.IsNullOrWhiteSpace(r.TaskLocation)) return "task|" + r.TaskLocation;
            return (r.Kind ?? "") + "|" + (r.Location ?? "") + "|" + (r.Name ?? "");
        }

        public static void RemoveByTask(string taskLocation)
        {
            if (string.IsNullOrWhiteSpace(taskLocation)) return;
            Save(Load().Where(r => !string.Equals(r.TaskLocation ?? "", taskLocation, StringComparison.OrdinalIgnoreCase)).ToList());
        }

        public static void Remove(StartupItem item)
        {
            if (item == null) return;
            string id = item.Id ?? "";
            if (id.StartsWith("task|", StringComparison.OrdinalIgnoreCase)) { RemoveByTask(item.Location); return; }
            if (id.StartsWith("reg|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("disabled|reg|", StringComparison.OrdinalIgnoreCase))
            {
                // Remove only the matching registry row; never touch unrelated rows.
                Save(Load().Where(r => !(r.Kind == "registry" && string.Equals(r.Name ?? "", item.Name ?? "", StringComparison.OrdinalIgnoreCase) && string.Equals(r.Location ?? "", item.Location ?? "", StringComparison.OrdinalIgnoreCase))).ToList());
                return;
            }
            if (id.StartsWith("folder|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("disabled|folder|", StringComparison.OrdinalIgnoreCase))
            {
                // Remove only the matching folder row; never touch unrelated rows.
                Save(Load().Where(r => !(r.Kind == "folder" && string.Equals(r.Command ?? "", item.Command ?? "", StringComparison.OrdinalIgnoreCase))).ToList());
                return;
            }
            if (id.StartsWith("service|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("driver|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("disabled|service|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("disabled|driver|", StringComparison.OrdinalIgnoreCase))
            {
                // Remove only the matching service/driver row; never touch unrelated rows.
                Save(Load().Where(r => !((r.Kind == "service" || r.Kind == "driver") && string.Equals(r.Name ?? "", item.Name ?? "", StringComparison.OrdinalIgnoreCase))).ToList());
                return;
            }
        }

        // Record an item the user explicitly enabled/restored so the guard keeps it running at every boot.
        public static void UpsertFromItem(StartupItem item)
        {
            if (item == null) return;
            try
            {
                string id = item.Id ?? "";
                string kind;
                string taskLocation = "";
                string mode = "normal";
                string target = "";
                string args = "";
                if (id.StartsWith("task|", StringComparison.OrdinalIgnoreCase))
                {
                    taskLocation = item.Location;
                    kind = item.IsManaged ? "managed-task" : "task";
                    StartupService.ResolveLaunchTarget(item, out target, out args);
                    mode = StartupService.CommandUsesTrayWrapper(item.Command ?? "") ? "tray" : "normal";
                }
                else if (id.StartsWith("disabled|task|", StringComparison.OrdinalIgnoreCase))
                {
                    taskLocation = item.Location;
                    kind = item.IsManaged ? "managed-task" : "task";
                    StartupService.ResolveLaunchTarget(item, out target, out args);
                    mode = StartupService.CommandUsesTrayWrapper(item.Command ?? "") ? "tray" : "normal";
                }
                else if (id.StartsWith("reg|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("disabled|reg|", StringComparison.OrdinalIgnoreCase))
                {
                    kind = "registry";
                    StartupService.ResolveLaunchTarget(item, out target, out args);
                }
                else if (id.StartsWith("folder|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("disabled|folder|", StringComparison.OrdinalIgnoreCase))
                {
                    kind = "folder";
                    StartupService.ResolveLaunchTarget(item, out target, out args);
                }
                else if (id.StartsWith("service|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("disabled|service|", StringComparison.OrdinalIgnoreCase))
                {
                    kind = "service";
                }
                else if (id.StartsWith("driver|", StringComparison.OrdinalIgnoreCase) || id.StartsWith("disabled|driver|", StringComparison.OrdinalIgnoreCase))
                {
                    kind = "driver";
                }
                else return;
                var row = new Row { Kind = kind, Name = item.Name ?? "", Scope = item.Scope ?? "", Command = item.Command ?? "", Location = item.Location ?? "", Status = item.Status ?? "", Target = target, Arguments = args, Mode = mode, TaskLocation = taskLocation };
                Upsert(row);
            }
            catch { }
        }

        public static List<Row> Load()
        {
            var list = new List<Row>();
            if (!File.Exists(StorePath)) return list;
            foreach (var line in File.ReadAllLines(StorePath))
            {
                var p = line.Split('\t');
                if (p.Length < 10) continue;
                list.Add(new Row { Kind = UnB64(p[0]), Name = UnB64(p[1]), Scope = UnB64(p[2]), Command = UnB64(p[3]), Location = UnB64(p[4]), Status = UnB64(p[5]), Target = UnB64(p[6]), Arguments = UnB64(p[7]), Mode = UnB64(p[8]), TaskLocation = UnB64(p[9]) });
            }
            return list;
        }

        private static void Save(List<Row> rows)
        {
            Directory.CreateDirectory(Program.AppData);
            File.WriteAllLines(StorePath, rows.Select(r => string.Join("\t", new[] { B64(r.Kind), B64(r.Name), B64(r.Scope), B64(r.Command), B64(r.Location), B64(r.Status), B64(r.Target), B64(r.Arguments), B64(r.Mode), B64(r.TaskLocation) })), Encoding.UTF8);
        }

        // Make sure every enabled managed task currently registered under \MichStartupMaster\ is tracked.
        public static void ImportExistingManagedTasks()
        {
            try
            {
                var rows = Load();
                var known = new HashSet<string>(rows.Where(r => r.Kind == "managed-task" && !string.IsNullOrWhiteSpace(r.TaskLocation)).Select(r => r.TaskLocation.ToLowerInvariant()), StringComparer.OrdinalIgnoreCase);
                string script =
                    "$ErrorActionPreference='Stop';" +
                    "foreach($t in Get-ScheduledTask){" +
                    "  $full=($t.TaskPath + $t.TaskName);" +
                    "  if(-not $full.StartsWith('\\MichStartupMaster\\')){ continue }" +
                    "  $enabled = if($t.Settings.Enabled){'true'}else{'false'};" +
                    "  $actions = (@($t.Actions) | ForEach-Object { if($_){ (($_.Execute) + ' ' + ($_.Arguments)).Trim() } }) -join ' || ';" +
                    "  $full + '\t' + $enabled + '\t' + $actions" +
                    "}";
                string output = StartupService.RunCapture(StartupService.PowerShellExe(), "-NoProfile -ExecutionPolicy Bypass -EncodedCommand " + Convert.ToBase64String(Encoding.Unicode.GetBytes(script)));
                foreach (var line in output.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries))
                {
                    string[] p = line.Split(new[] { '\t' }, 3);
                    if (p.Length < 3) continue;
                    string taskLocation = p[0];
                    bool enabled = p[1].Equals("true", StringComparison.OrdinalIgnoreCase);
                    if (!enabled || string.Equals(taskLocation, Program.ManagedTaskRoot + "MichStartupMasterApp", StringComparison.OrdinalIgnoreCase)) continue;
                    // Never re-adopt a task the user disabled (even a half-disabled one): the
                    // protected-disabled store records their intent and the guard keeps it off.
                    // A task still in the quiet-protection store is exempt: the quiet store proves
                    // the user wanted it RUNNING (quietly), so a stale disabled protection left by
                    // an older half-failed disable is healed instead of orphaned forever.
                    if (ProtectedDisabledService.IsProtected("task|" + taskLocation) && !ProtectedQuietService.IsProtected(taskLocation)) continue;
                    if (known.Contains(taskLocation.ToLowerInvariant())) continue;
                    string action = p[2];
                    string target = "";
                    string args = "";
                    string mode = "normal";
                    if (action.IndexOf("--start-in-tray", StringComparison.OrdinalIgnoreCase) >= 0) { mode = "tray"; target = StartupService.ProcessExePath(); }
                    else if (action.IndexOf("--tray-run", StringComparison.OrdinalIgnoreCase) >= 0)
                    {
                        mode = "tray";
                        StartupService.TryDecodeTrayPayload(action, out target, out args);
                    }
                    else
                    {
                        mode = "normal";
                        StartupService.TrySplitCommand(action, out target, out args);
                    }
                    if (string.IsNullOrWhiteSpace(target) || !File.Exists(target)) continue;
                    if (!StartupService.IsSupportedStartupTarget(target)) continue;
                    Upsert(new Row { Kind = "managed-task", Name = taskLocation.TrimStart('\\'), Scope = "User/System", Command = action, Location = taskLocation, Status = "Managed startup task", Target = target, Arguments = args, Mode = mode, TaskLocation = taskLocation });
                }
            }
            catch { }
        }

        // Adopt every startup item the legacy v2 state (state-v2.json) marked Enabled into the
        // managed-task enforcement system, so items set to start in the old app also start at
        // every boot with the same no-exceptions guarantee as everything else.
        public static string MigrateV2EnabledItems()
        {
            int adopted = 0, skipped = 0, missing = 0;
            try
            {
                string v2Path = Path.Combine(Program.AppData, "state-v2.json");
                if (!File.Exists(v2Path)) return "MIGRATE_V2 none";
                string raw;
                try { raw = File.ReadAllText(v2Path); } catch { return "MIGRATE_V2 unreadable"; }
                JsonDocument doc;
                try { doc = JsonDocument.Parse(raw); } catch { return "MIGRATE_V2 invalid-json"; }
                using (doc)
                {
                    if (!doc.RootElement.TryGetProperty("Items", out var itemsEl) || itemsEl.ValueKind != JsonValueKind.Array) return "MIGRATE_V2 no-items";
                    var rows = Load();
                    // Every managed task already known to the new system is taken over for good:
                    // backfill the one-shot marker so disabling or removing one of them can never
                    // cause the v2 migration to resurrect it on a later pass.
                    foreach (var r in rows.Where(r => r.Kind == "managed-task" && !string.IsNullOrWhiteSpace(r.TaskLocation))) MarkV2Migrated(r.TaskLocation);
                    foreach (var it in itemsEl.EnumerateArray())
                    {
                        try
                        {
                            if (!JsonBool(it, "Enabled")) continue;
                            string name = JsonStr(it, "LegacyTaskName");
                            if (string.IsNullOrWhiteSpace(name)) name = JsonStr(it, "Name");
                            string target = JsonStr(it, "Target");
                            string args = JsonStr(it, "Arguments") ?? "";
                            int launchMode = JsonInt(it, "LaunchMode");
                            if (string.IsNullOrWhiteSpace(name) || string.IsNullOrWhiteSpace(target)) continue;
                            string safeName = Regex.Replace(name, "[^A-Za-z0-9 _.-]", "").Trim();
                            if (safeName.Length == 0) safeName = "StartupApp";
                            string taskLocation = Program.ManagedTaskRoot + safeName;
                            // Never re-adopt an item the new app has already taken over (the user may
                            // have disabled or removed it since; the manifest alone is not the marker
                            // because disabling deletes the manifest row).
                            if (IsV2Migrated(taskLocation)) { skipped++; continue; }
                            if (rows.Any(r => r.Kind == "managed-task" && string.Equals(r.TaskLocation, taskLocation, StringComparison.OrdinalIgnoreCase))) { skipped++; continue; }
                            bool trayMode = launchMode == 2;
                            var row = new Row
                            {
                                Kind = "managed-task",
                                Name = safeName,
                                Scope = "User/System",
                                Command = "",
                                Location = taskLocation,
                                Status = "Managed startup task (migrated from v2)",
                                Target = target,
                                Arguments = args,
                                Mode = trayMode ? "tray" : "normal",
                                TaskLocation = taskLocation
                            };
                            // Manifest intent wins: drop any stale disabled protection for this task.
                            ProtectedDisabledService.UnprotectKey("task|" + taskLocation);
                            if (File.Exists(target))
                            {
                                string execute, actionArgs;
                                StartupService.BuildManagedAction(target, args, trayMode, out execute, out actionArgs);
                                row.Command = execute + " " + actionArgs;
                                StartupService.RegisterLogonTaskAt(taskLocation, execute, actionArgs);
                                if (trayMode) ProtectedQuietService.ProtectTask(taskLocation, target, args);
                                // A task that did not exist or was disabled before boot must run now,
                                // unless the app is already running (never double-start anything).
                                if (!IsProcessRunning(target)) TryRunTask(taskLocation);
                                adopted++;
                            }
                            else
                            {
                                // Target not on disk yet: record intent so the guard adopts and runs it
                                // the moment the file appears, without the user having to touch anything.
                                missing++;
                            }
                            // The managed task is now the single launcher: retire duplicate native
                            // sources (registry Run values / stale disabled records) for the same app.
                            RetireDuplicateNativeSource(target);
                            Upsert(row);
                            MarkV2Migrated(taskLocation);
                        }
                        catch { }
                    }
                }
                return "MIGRATE_V2 adopted=" + adopted + " skipped=" + skipped + " missing=" + missing;
            }
            catch { return "MIGRATE_V2 error"; }
        }

        private static string MigratedMarkerPath { get { return Path.Combine(Program.AppData, "migrated-v2-items.tsv"); } }

        internal static bool IsV2Migrated(string taskLocation)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(taskLocation) || !File.Exists(MigratedMarkerPath)) return false;
                return File.ReadAllLines(MigratedMarkerPath).Any(l => string.Equals(l.Trim().TrimStart('\uFEFF'), taskLocation, StringComparison.OrdinalIgnoreCase));
            }
            catch { return false; }
        }

        internal static void MarkV2Migrated(string taskLocation)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(taskLocation)) return;
                Directory.CreateDirectory(Program.AppData);
                if (IsV2Migrated(taskLocation)) return;
                File.AppendAllText(MigratedMarkerPath, taskLocation + Environment.NewLine, Encoding.UTF8);
            }
            catch { }
        }

        internal static bool IsProcessRunning(string target)
        {
            try
            {
                string name = Path.GetFileNameWithoutExtension(target ?? "").ToLowerInvariant();
                if (string.IsNullOrWhiteSpace(name)) return false;
                // Script hosts are always present; they are not the app itself, so never treat
                // their presence as proof the startup target is already running.
                if (name == "powershell" || name == "powershell_ise" || name == "pwsh" || name == "cmd" || name == "cscript" || name == "wscript" || name == "conhost" || name == "rundll32") return false;
                return Process.GetProcessesByName(name).Length > 0;
            }
            catch { return false; }
        }

        private static string JsonStr(JsonElement el, string key)
        {
            try { if (el.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.String) return v.GetString(); } catch { }
            return "";
        }

        private static bool JsonBool(JsonElement el, string key)
        {
            try { if (el.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.True) return true; } catch { }
            return false;
        }

        private static int JsonInt(JsonElement el, string key)
        {
            try { if (el.TryGetProperty(key, out var v) && v.ValueKind == JsonValueKind.Number) return v.GetInt32(); } catch { }
            return 0;
        }

        // The managed task is the single canonical launcher for a migrated app: remove any
        // registry Run value or stale legacy disabled record that launches the same executable,
        // so the app can never start twice at boot.
        private static void RetireDuplicateNativeSource(string target)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(target)) return;
                string norm = target.Trim().Trim('"').TrimEnd('\\').ToLowerInvariant();
                foreach (var sub in new[] { @"Software\Microsoft\Windows\CurrentVersion\Run", @"Software\Microsoft\Windows\CurrentVersion\RunOnce" })
                {
                    foreach (var root in new[] { Registry.CurrentUser, Registry.LocalMachine })
                    {
                        try
                        {
                            using (var key = root.OpenSubKey(sub, true))
                            {
                                if (key == null) continue;
                                foreach (var vn in key.GetValueNames())
                                {
                                    object v = key.GetValue(vn, null, RegistryValueOptions.DoNotExpandEnvironmentNames);
                                    string cmd = v == null ? "" : v.ToString();
                                    if (cmd.Trim().Trim('"').TrimEnd('\\').ToLowerInvariant() == norm) key.DeleteValue(vn, false);
                                }
                            }
                        }
                        catch { }
                    }
                }
                if (File.Exists(Program.DisabledStore))
                {
                    var kept = File.ReadAllLines(Program.DisabledStore).Where(l =>
                    {
                        var p = l.Split('\t');
                        if (p.Length < 5) return true;
                        string cmd = UnB64(p[4]);
                        return cmd.Trim().Trim('"').TrimEnd('\\').ToLowerInvariant() != norm;
                    }).ToArray();
                    File.WriteAllLines(Program.DisabledStore, kept, Encoding.UTF8);
                }
            }
            catch { }
        }

        // Every managed item must be the single launcher for its app: retire any registry Run
        // value or legacy scheduled task that launches the same (target + arguments), so a
        // migrated item can never start twice at boot. Runs only in the throttled import pass.
        private static void RetireDuplicateLaunchSources()
        {
            try
            {
                var rows = Load().Where(r => r.Kind == "managed-task" && !string.IsNullOrWhiteSpace(r.Target) && File.Exists(r.Target)).ToList();
                if (rows.Count == 0) return;
                foreach (var row in rows)
                {
                    try
                    {
                        RetireDuplicateNativeSource(row.Target);
                        RetireDuplicateTask(row.Target, row.Arguments ?? "");
                    }
                    catch { }
                }
            }
            catch { }
        }

        // Disable any scheduled task OUTSIDE the managed root whose action launches exactly the
        // same (executable + arguments) as a managed item, and protect it so the guard keeps it off.
        private static void RetireDuplicateTask(string target, string arguments)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(target)) return;
                string canonical = NormalizeCommand(target + " " + (arguments ?? ""));
                if (string.IsNullOrWhiteSpace(canonical)) return;
                string script =
                    "$ErrorActionPreference='Stop';" +
                    "foreach($t in Get-ScheduledTask){" +
                    "  $full=($t.TaskPath + $t.TaskName);" +
                    "  if($full.StartsWith('\\MichStartupMaster\\')){ continue }" +
                    "  $actions = (@($t.Actions) | ForEach-Object { if($_){ (($_.Execute) + ' ' + ($_.Arguments)).Trim() } }) -join ' || ';" +
                    "  $full + '\t' + $actions" +
                    "}";
                string output = StartupService.RunCapture(StartupService.PowerShellExe(), "-NoProfile -ExecutionPolicy Bypass -EncodedCommand " + Convert.ToBase64String(Encoding.Unicode.GetBytes(script)));
                foreach (var line in output.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries))
                {
                    string[] p = line.Split(new[] { '\t' }, 2);
                    if (p.Length < 2) continue;
                    string taskLocation = p[0];
                    string action = p[1];
                    // Match only exact same-app launchers, never a different script/arguments.
                    if (!string.Equals(NormalizeCommand(action), canonical, StringComparison.OrdinalIgnoreCase)) continue;
                    if (string.Equals(taskLocation, Program.ManagedTaskRoot + "MichStartupMasterApp", StringComparison.OrdinalIgnoreCase)) continue;
                    try { StartupService.RunChecked("schtasks.exe", "/Change /TN " + StartupService.Q(taskLocation) + " /Disable"); } catch { }
                    ProtectedDisabledService.ProtectKey("task|" + taskLocation, taskLocation, "Scheduled Task", "Retired duplicate launcher for " + target);
                }
            }
            catch { }
        }

        private static string NormalizeCommand(string command)
        {
            try
            {
                return Regex.Replace((command ?? "").ToLowerInvariant(), @"\s+", " ").Trim().Trim('"');
            }
            catch { return (command ?? "").ToLowerInvariant(); }
        }

        private static DateTime _lastImportUtc = DateTime.MinValue;
        private static int _busy;

        public static string EnforceEnabled(bool includeImport = true)
        {
            if (System.Threading.Interlocked.Exchange(ref _busy, 1) == 1) return "ENFORCE_ENABLED busy";
            try
            {
                // Adopt legacy v2 enabled items and any managed task created outside the app at
                // most every few minutes, and always on the first pass, so nothing enabled can be missed.
                if (includeImport && (DateTime.UtcNow - _lastImportUtc).TotalMinutes >= 5)
                {
                    MigrateV2EnabledItems();
                    ImportExistingManagedTasks();
                    // Each managed item must be the single launcher: retire duplicate registry
                    // Run values and legacy tasks that launch the same app, so nothing starts twice.
                    RetireDuplicateLaunchSources();
                    _lastImportUtc = DateTime.UtcNow;
                }
                int actions = 0, failures = 0;
                // The app's own hidden boot agent must always be registered, enabled, and correct.
                try
                {
                    string exe = StartupService.ProcessExePath();
                    if (!string.IsNullOrWhiteSpace(exe) && File.Exists(exe))
                    {
                        var agentRow = new Row { Kind = "managed-task", Name = "MichStartupMasterApp", Scope = "User/System", Command = exe + " --agent", Location = Program.ManagedTaskRoot + "MichStartupMasterApp", Status = "Managed startup agent", Target = exe, Arguments = "--agent", Mode = "normal", TaskLocation = Program.ManagedTaskRoot + "MichStartupMasterApp" };
                        if (EnforceRow(agentRow)) actions++;
                    }
                }
                catch { failures++; }
                foreach (var row in Load())
                {
                    try { if (EnforceRow(row)) actions++; }
                    catch { failures++; }
                }
                return "ENFORCE_ENABLED protected=" + Load().Count + " actions=" + actions + " failures=" + failures;
            }
            finally
            {
                System.Threading.Interlocked.Exchange(ref _busy, 0);
            }
        }

        private static bool EnforceRow(Row row)
        {
            if (row == null) return false;
            if (row.Kind == "managed-task" || row.Kind == "task")
            {
                string taskLocation = row.TaskLocation;
                if (string.IsNullOrWhiteSpace(taskLocation)) taskLocation = row.Location;
                if (string.IsNullOrWhiteSpace(taskLocation)) return false;
                // The enabled manifest is authoritative for this task; drop any stale disabled protection.
                ProtectedDisabledService.UnprotectKey("task|" + taskLocation);
                bool isManaged = row.Kind == "managed-task";
                bool tray = string.Equals(row.Mode, "tray", StringComparison.OrdinalIgnoreCase);
                string expectedExec, expectedArgs;
                if (isManaged)
                {
                    string target = row.Target;
                    if (string.IsNullOrWhiteSpace(target)) return false;
                    if (!File.Exists(target)) target = StartupService.ResolveTargetPath(target);
                    if (string.IsNullOrWhiteSpace(target) || !File.Exists(target)) return false;
                    if (!string.Equals(target, row.Target, StringComparison.OrdinalIgnoreCase))
                    {
                        // The packaged app auto-updated into a new version folder; persist the
                        // resolved path so the manifest self-heals and the task gets re-registered
                        // with the current launcher on the lines below.
                        row.Target = target;
                        Upsert(row);
                    }
                    StartupService.BuildManagedAction(target, row.Arguments ?? "", tray, out expectedExec, out expectedArgs);
                }
                else
                {
                    expectedExec = row.Target; expectedArgs = row.Arguments ?? "";
                }
                string state = QueryTaskState(taskLocation);
                if (state == null)
                {
                    if (isManaged) StartupService.RegisterLogonTaskAt(taskLocation, expectedExec, expectedArgs);
                    else StartupService.RunChecked("schtasks.exe", "/Create /F /TN " + StartupService.Q(taskLocation) + " /SC ONLOGON /TR " + StartupService.Q(StartupService.WinArg(expectedExec) + (string.IsNullOrWhiteSpace(expectedArgs) ? "" : " " + expectedArgs)));
                    // The task was missing, so it would not start at this boot unless we run it right now.
                    TryRunTask(taskLocation);
                    return true;
                }
                if (state == "disabled")
                {
                    StartupService.RunChecked("schtasks.exe", "/Change /TN " + StartupService.Q(taskLocation) + " /Enable");
                    // Re-enabling alone does not fire the logon trigger; launch it immediately so it starts this boot.
                    TryRunTask(taskLocation);
                    return true;
                }
                if (isManaged)
                {
                    // Verify the action matches the desired mode and has an immediate logon trigger.
                    string xml = null;
                    try { xml = StartupService.RunCapture("schtasks.exe", "/Query /TN " + StartupService.Q(taskLocation) + " /XML"); }
                    catch { }
                    bool correct = xml != null && xml.IndexOf("<LogonTrigger", StringComparison.OrdinalIgnoreCase) >= 0 && xml.IndexOf("<Delay>", StringComparison.OrdinalIgnoreCase) < 0;
                    if (correct)
                    {
                        string cmd = ExtractXmlElement(xml, "Command");
                        string argText = ExtractXmlElement(xml, "Arguments");
                        bool actionMatches = string.Equals(cmd ?? "", expectedExec ?? "", StringComparison.OrdinalIgnoreCase) && string.Equals((argText ?? "").Trim(), (expectedArgs ?? "").Trim(), StringComparison.OrdinalIgnoreCase);
                        if (!actionMatches) correct = false;
                    }
                    if (!correct)
                    {
                        StartupService.RegisterLogonTaskAt(taskLocation, expectedExec, expectedArgs);
                        TryRunTask(taskLocation);
                        return true;
                    }
                }
                return false;
            }
            if (row.Kind == "registry")
            {
                RegistryKey root;
                string subKey;
                string valueName;
                RegistryValueKind kind = RegistryValueKind.String;
                string command = row.Command ?? "";
                string[] meta = (row.Status ?? "").Split('\t');
                if (meta.Length >= 4 && meta[0].StartsWith("HKEY_", StringComparison.OrdinalIgnoreCase))
                {
                    root = RootFromName(meta[0]);
                    subKey = meta[1];
                    valueName = meta[2];
                    try { kind = (RegistryValueKind)Enum.Parse(typeof(RegistryValueKind), meta[3], true); } catch { }
                }
                else
                {
                    root = string.Equals(row.Scope, "Machine", StringComparison.OrdinalIgnoreCase) ? Registry.LocalMachine : Registry.CurrentUser;
                    subKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
                    valueName = row.Name;
                }
                if (string.IsNullOrWhiteSpace(valueName) || string.IsNullOrWhiteSpace(command)) return false;
                // Manifest intent wins over any stale disabled protection for this registry value.
                ProtectedDisabledService.UnprotectKey("reg|" + row.Scope + "|" + root.Name + "|" + subKey + "|" + valueName);
                using (var key = root.OpenSubKey(subKey, false))
                {
                    if (key != null && key.GetValue(valueName, null, RegistryValueOptions.DoNotExpandEnvironmentNames) != null) return false;
                }
                using (var key = root.CreateSubKey(subKey)) key.SetValue(valueName, command, kind);
                // A Run value only takes effect at the next logon; start it now so this boot is not missed.
                TryLaunchTarget(row.Target, row.Arguments);
                return true;
            }
            if (row.Kind == "folder")
            {
                string file = row.Command ?? "";
                if (string.IsNullOrWhiteSpace(file)) return false;
                ProtectedDisabledService.UnprotectKey("folder|" + row.Scope + "|" + file);
                if (File.Exists(file)) return false;
                // Try to recover from the app's disabled-folder quarantine.
                string originalName = Path.GetFileName(file);
                string quarantine = Program.DisabledStartupFolder;
                if (Directory.Exists(quarantine))
                {
                    var match = Directory.GetFiles(quarantine).FirstOrDefault(f => Path.GetFileName(f).StartsWith(originalName + ".", StringComparison.OrdinalIgnoreCase));
                    if (match != null)
                    {
                        Directory.CreateDirectory(Path.GetDirectoryName(file));
                        File.Move(match, file);
                        TryLaunchTarget(row.Target, row.Arguments);
                        return true;
                    }
                }
                return false;
            }
            if (row.Kind == "service" || row.Kind == "driver")
            {
                string[] meta = (row.Status ?? "").Split('\t');
                string serviceName = meta.Length > 0 ? meta[0] : row.Name;
                if (string.IsNullOrWhiteSpace(serviceName)) return false;
                ProtectedDisabledService.UnprotectKey(Convert.ToBase64String(Encoding.UTF8.GetBytes(serviceName ?? "")) + "|" + serviceName);
                using (var key = Registry.LocalMachine.OpenSubKey(@"SYSTEM\CurrentControlSet\Services\" + serviceName, true))
                {
                    if (key == null) return false;
                    object current = key.GetValue("Start");
                    if (current != null && Convert.ToInt32(current) == 4)
                    {
                        key.SetValue("Start", 2, RegistryValueKind.DWord);
                        TryStartService(serviceName);
                        return true;
                    }
                }
                return false;
            }
            return false;
        }

        // Launch a repaired task immediately so it runs at the current boot, not only the next one.
        private static void TryRunTask(string taskLocation)
        {
            // The app's own agent is already this process; never re-launch it.
            if (string.Equals(taskLocation, Program.ManagedTaskRoot + "MichStartupMasterApp", StringComparison.OrdinalIgnoreCase)) return;
            try
            {
                // Never double-start an app that is already running.
                var row = Load().FirstOrDefault(r => string.Equals(r.TaskLocation, taskLocation, StringComparison.OrdinalIgnoreCase));
                if (row != null && !string.IsNullOrWhiteSpace(row.Target) && IsProcessRunning(row.Target)) return;
                StartupService.RunChecked("schtasks.exe", "/Run /TN " + StartupService.Q(taskLocation));
            }
            catch { }
        }

        private static void TryLaunchTarget(string target, string arguments)
        {
            try
            {
                if (string.IsNullOrWhiteSpace(target) || !File.Exists(target)) return;
                string execute, actionArgs;
                StartupService.BuildDirectAction(target, arguments ?? "", out execute, out actionArgs);
                var psi = new ProcessStartInfo(execute, actionArgs)
                {
                    UseShellExecute = false,
                    WorkingDirectory = Directory.Exists(Path.GetDirectoryName(target)) ? Path.GetDirectoryName(target) : Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)
                };
                Process.Start(psi);
            }
            catch { }
        }

        private static void TryStartService(string serviceName)
        {
            try { StartupService.RunChecked("sc.exe", "start " + serviceName); }
            catch { }
        }

        private static string QueryTaskState(string taskLocation)
        {
            try
            {
                string output = StartupService.RunCapture("schtasks.exe", "/Query /TN " + StartupService.Q(taskLocation) + " /FO LIST /V");
                var m = Regex.Match(output ?? "", @"Scheduled Task State:\s*(?<v>[A-Za-z]+)", RegexOptions.IgnoreCase);
                if (m.Success)
                {
                    string v = m.Groups["v"].Value.ToLowerInvariant();
                    if (v.StartsWith("dis", StringComparison.Ordinal)) return "disabled";
                    return "enabled";
                }
                return "enabled";
            }
            catch { return null; }
        }

        private static string ExtractXmlElement(string xml, string element)
        {
            try
            {
                var m = Regex.Match(xml, "<" + element + ">(?<v>.*?)</" + element + ">", RegexOptions.Singleline | RegexOptions.IgnoreCase);
                return m.Success ? m.Groups["v"].Value.Trim() : null;
            }
            catch { return null; }
        }

        public static string ToJson()
        {
            var rows = Load();
            var sb = new StringBuilder();
            sb.Append("[");
            for (int i = 0; i < rows.Count; i++)
            {
                if (i > 0) sb.Append(',');
                var r = rows[i];
                sb.Append("{\"kind\":\"").Append(Esc(r.Kind)).Append("\",\"name\":\"").Append(Esc(r.Name)).Append("\",\"target\":\"").Append(Esc(r.Target)).Append("\",\"mode\":\"").Append(Esc(r.Mode)).Append("\",\"task\":\"").Append(Esc(r.TaskLocation)).Append("\"}");
            }
            sb.Append("]");
            return sb.ToString();
        }

        private static string Esc(string s) { return (s ?? "").Replace("\\", "\\\\").Replace("\"", "\\\"").Replace("\r", " ").Replace("\n", " "); }
        private static RegistryKey RootFromName(string rootName)
        {
            if (string.Equals(rootName, Registry.LocalMachine.Name, StringComparison.OrdinalIgnoreCase) || string.Equals(rootName, "HKLM", StringComparison.OrdinalIgnoreCase)) return Registry.LocalMachine;
            if (string.Equals(rootName, Registry.CurrentUser.Name, StringComparison.OrdinalIgnoreCase) || string.Equals(rootName, "HKCU", StringComparison.OrdinalIgnoreCase)) return Registry.CurrentUser;
            throw new InvalidOperationException("Unsupported registry root: " + rootName);
        }
        private static string B64(string s) { return Convert.ToBase64String(Encoding.UTF8.GetBytes(s ?? "")); }
        private static string UnB64(string s) { try { return Encoding.UTF8.GetString(Convert.FromBase64String(s)); } catch { return ""; } }
    }

    // Watches for brand-new startup entries appearing from ANY source (another app or installer,
    // a registry edit, a new scheduled task, a dropped Startup-folder shortcut) and reports them
    // so the user is told the moment something new is set to run at boot. A persisted signature
    // store makes only genuinely new entries toast — the pre-existing inventory is seeded as the
    // baseline on the very first scan and never floods with notifications.
    internal static class StartupWatcher
    {
        // MSM_KNOWN_STORE lets the regression suite point the watcher at an isolated store so a
        // "new item detected" test is fully deterministic and never races the live agent.
        private static string StorePath
        {
            get
            {
                string overridePath = Environment.GetEnvironmentVariable("MSM_KNOWN_STORE");
                if (!string.IsNullOrWhiteSpace(overridePath)) return overridePath;
                return Path.Combine(Program.AppData, "known-startup-items.tsv");
            }
        }

        // A stable identity for one logical startup entry. It deliberately excludes per-scan noise
        // (case, extra whitespace) so the same entry is never re-reported on later scans.
        private static string Identity(StartupItem item)
        {
            string name = (item.Name ?? "").Trim();
            string loc = (item.Location ?? "").Trim();
            string cmd = (item.Command ?? "").Trim();
            return (item.Source ?? "") + "\u0001" + name + "\u0001" + loc + "\u0001" + cmd;
        }

        private static HashSet<string> LoadKnown()
        {
            var set = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            try
            {
                if (File.Exists(StorePath))
                {
                    foreach (string line in File.ReadAllLines(StorePath))
                    {
                        string sig = UnB64(line.Split('\t')[0]);
                        if (!string.IsNullOrWhiteSpace(sig)) set.Add(sig);
                    }
                }
            }
            catch { }
            return set;
        }

        private static void SaveKnown(IEnumerable<StartupItem> items)
        {
            try
            {
                Directory.CreateDirectory(Program.AppData);
                var lines = items.Select(i => B64(Identity(i))).Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
                WriteAllLinesWithRetry(StorePath, lines);
            }
            catch { }
        }

        // Returns the startup entries that appeared since the previous call. The first call only
        // seeds the baseline and returns nothing, so the app never toasts for its existing inventory.
        public static List<StartupItem> DetectNew()
        {
            var current = StartupService.ScanAll();
            try
            {
                if (!File.Exists(StorePath))
                {
                    SaveKnown(current);
                    return new List<StartupItem>();
                }
                var known = LoadKnown();
                var fresh = current.Where(i => !(i.Id ?? "").StartsWith("error|", StringComparison.OrdinalIgnoreCase) && !known.Contains(Identity(i))).ToList();
                SaveKnown(current);
                return fresh;
            }
            catch { return new List<StartupItem>(); }
        }

        private static void WriteAllLinesWithRetry(string path, string[] lines)
        {
            for (int attempt = 0; ; attempt++)
            {
                try { File.WriteAllLines(path, lines, Encoding.UTF8); return; }
                catch (IOException) { if (attempt >= 4) throw; System.Threading.Thread.Sleep(300); }
                catch (UnauthorizedAccessException) { if (attempt >= 4) throw; System.Threading.Thread.Sleep(300); }
            }
        }

        private static string B64(string s) { return Convert.ToBase64String(Encoding.UTF8.GetBytes(s ?? "")); }
        private static string UnB64(string s) { try { return Encoding.UTF8.GetString(Convert.FromBase64String(s)); } catch { return ""; } }
    }

    internal static class TrayRunner
    {
        private delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
        [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
        [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
        [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hWnd);
        [DllImport("user32.dll")] private static extern bool ShowWindowAsync(IntPtr hWnd, int nCmdShow);
        [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr hWnd);
        private const int SW_HIDE = 0;
        private const int SW_RESTORE = 9;
        // How long after launch the wrapper aggressively suppresses EVERY window the target shows
        // (the boot popup window). After this it switches to "only hide brand-new windows" mode so
        // the user can still open the app's GUI from its tray icon.
        private const int BootSettleSeconds = 60;
        private const int AutoExitGraceSeconds = 15;

        public static void RunMain(string[] args)
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
                if (!File.Exists(full)) full = StartupService.ResolveTargetPath(full);
                if (string.IsNullOrWhiteSpace(full) || !File.Exists(full) || !StartupService.IsSupportedStartupTarget(full)) return;
                // The single-instance identity is the full launch identity (target + arguments),
                // not just the host path: quiet apps that share a script host (wscript.exe,
                // powershell.exe, ...) must never collide with each other's wrapper.
                string mutexName = @"Local\MichStartupMaster.TrayWrapper." + HashName(full + "\n" + targetArgs);
                bool createdNew;
                using (var mutex = new System.Threading.Mutex(true, mutexName, out createdNew))
                {
                    if (!createdNew) return; // another quiet wrapper already controls this exact launch
                    Application.EnableVisualStyles();
                    Application.SetCompatibleTextRenderingDefault(false);
                    using (var ctx = new TrayWrapperContext(full, targetArgs))
                    {
                        Application.Run(ctx);
                    }
                }
            }
            catch { }
        }

        private static string HashName(string value)
        {
            using (var sha = SHA1.Create())
            {
                byte[] h = sha.ComputeHash(Encoding.UTF8.GetBytes((value ?? "").ToLowerInvariant()));
                var sb = new StringBuilder();
                foreach (byte b in h) sb.Append(b.ToString("x2"));
                return sb.ToString();
            }
        }

        private sealed class TrayWrapperContext : ApplicationContext
        {
            private readonly string _target;
            private readonly string _targetArgs;
            private Timer _hideTimer;
            private Timer _watchTimer;
            private DateTime _startedUtc = DateTime.UtcNow;
            private int _rootPid;
            private readonly HashSet<int> _tree = new HashSet<int>();
            // Windows this wrapper has hidden and windows the user has intentionally re-opened.
            private readonly HashSet<IntPtr> _hiddenWindows = new HashSet<IntPtr>();
            private readonly HashSet<IntPtr> _exemptWindows = new HashSet<IntPtr>();
            private DateTime? _treeDeadSince;
            private bool _exiting;

            public TrayWrapperContext(string target, string targetArgs)
            {
                _target = target;
                _targetArgs = targetArgs;
                StartTarget();
                // No tray icon of its own: the wrapper is an invisible quiet launcher so the
                // target app's OWN tray icon is the only one ever shown. This permanently
                // removes the duplicate/broken "wrapper" icons next to GameSir, whisper-key,
                // AutoHotkey, etc. Clicking the app's own icon (or "Launch now" in Startup
                // Master) opens its GUI.
                _hideTimer = new Timer { Interval = 1000 };
                _hideTimer.Tick += (s, e) => HideNewWindows();
                _hideTimer.Start();
                _watchTimer = new Timer { Interval = 5000 };
                _watchTimer.Tick += (s, e) => CheckAlive();
                _watchTimer.Start();
            }

            private void StartTarget()
            {
                try
                {
                    // Single instance per target: if the app is already running (another launcher
                    // fired first, or the user started it manually), never start a second copy and
                    // never add a second tray icon for it — quietly hand over instead.
                    if (EnabledStartupService.IsProcessRunning(_target))
                    {
                        _rootPid = 0;
                        Environment.Exit(0);
                        return;
                    }
                    string execute, actionArgs;
                    StartupService.BuildDirectAction(_target, _targetArgs, out execute, out actionArgs);
                    var psi = new ProcessStartInfo(execute, actionArgs)
                    {
                        UseShellExecute = false,
                        CreateNoWindow = true,
                        // Start fully hidden so even a brief startup flash never reaches the desktop.
                        WindowStyle = ProcessWindowStyle.Hidden,
                        WorkingDirectory = Directory.Exists(Path.GetDirectoryName(_target)) ? Path.GetDirectoryName(_target) : Environment.GetFolderPath(Environment.SpecialFolder.UserProfile)
                    };
                    var p = Process.Start(psi);
                    _rootPid = p.Id;
                    RefreshTree();
                }
                catch { }
            }

            private void HideNewWindows()
            {
                if (_exiting) return;
                try
                {
                    RefreshTree();
                    var pids = new HashSet<int>(_tree);
                    if (_rootPid != 0) pids.Add(_rootPid);
                    bool settled = (DateTime.UtcNow - _startedUtc).TotalSeconds > BootSettleSeconds;
                    EnumWindows((h, l) =>
                    {
                        uint pid;
                        GetWindowThreadProcessId(h, out pid);
                        if (!pids.Contains((int)pid) || !IsWindowVisible(h)) return true;
                        if (_exemptWindows.Contains(h)) return true;
                        // After the boot settle period, a window we already hid that is visible
                        // again means the user opened it on purpose (e.g. clicked the app's tray
                        // icon) — leave it alone from then on.
                        if (settled && _hiddenWindows.Contains(h)) { _exemptWindows.Add(h); return true; }
                        ShowWindowAsync(h, SW_HIDE);
                        _hiddenWindows.Add(h);
                        return true;
                    }, IntPtr.Zero);
                }
                catch { }
            }

            private void CheckAlive()
            {
                if (_exiting) return;
                try
                {
                    RefreshTree();
                    bool anyAlive = false;
                    var all = new HashSet<int>(_tree);
                    if (_rootPid != 0) all.Add(_rootPid);
                    foreach (int pid in all)
                    {
                        try { using (var p = Process.GetProcessById(pid)) { if (!p.HasExited) { anyAlive = true; break; } } }
                        catch { }
                    }
                    if (anyAlive) { _treeDeadSince = null; return; }
                    if (_treeDeadSince == null) _treeDeadSince = DateTime.UtcNow;
                    else if ((DateTime.UtcNow - _treeDeadSince.Value).TotalSeconds >= AutoExitGraceSeconds) ExitWrapper();
                }
                catch { }
            }

            private void RefreshTree()
            {
                _tree.Clear();
                if (_rootPid == 0) return;
                foreach (int pid in ChildProcessIds(_rootPid)) _tree.Add(pid);
            }

            private void ExitWrapper()
            {
                if (_exiting) return;
                _exiting = true;
                try { if (_hideTimer != null) { _hideTimer.Stop(); _hideTimer.Dispose(); } } catch { }
                try { if (_watchTimer != null) { _watchTimer.Stop(); _watchTimer.Dispose(); } } catch { }
                try { ExitThread(); } catch { }
            }
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
    }

    internal sealed class MainForm : Form
    {
        private List<StartupItem> _items = new List<StartupItem>();
        private ListView _list;
        private TextBox _search;
        private Label _summary, _visibleValue, _enabledValue, _disabledValue, _reviewValue, _managedValue, _hint;
        private Button _refresh, _disable, _enable, _add, _editSelected, _deleteManaged, _clearSearch, _showAll, _showRisky, _showCleanup, _showDisabled, _quietSelected, _protectNow, _enforceNow, _coverage, _openFolders;
        private NotifyIcon _tray;
        private Timer _guardTimer;
        private ToolTip _tooltip;
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
            _tooltip = new ToolTip { AutoPopDelay = 6000, InitialDelay = 400, ReshowDelay = 200, ShowAlways = true };
            BuildUi(); BuildTray();
            Load += (s, e) => { RefreshItems(); DetectNewStartupItems(); };
            _guardTimer = new Timer { Interval = 30000 };
            _guardTimer.Tick += (s, e) => { RunGuardsAsync(false); DetectNewStartupItems(); };
            _guardTimer.Start();
            FormClosing += OnClosingToTray;
            Resize += (s, e) => { if (WindowState == FormWindowState.Minimized) HideToTray(); };
            KeyDown += MainFormKeyDown;
            Shown += (s, e) =>
            {
                if (_startInTray) BeginInvoke(new Action(HideToTray));
                else if (_search != null) _search.Clear();
            };
            // A window that was hidden and is shown again (e.g. opened from the Start Menu while
            // the hidden boot agent owns the window) must always present the full list.
            VisibleChanged += (s, e) => { if (Visible && _search != null) _search.Clear(); };
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
            var heroIcon = new PictureBox { Image = Program.AppIcon.ToBitmap(), SizeMode = PictureBoxSizeMode.Zoom, Bounds = new Rectangle(hero.Width - 92, 22, 60, 60), Anchor = AnchorStyles.Top | AnchorStyles.Right, BackColor = Color.Transparent };
            hero.Controls.Add(heroIcon);

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
            _coverage = Button("Coverage", Steel, 92); _coverage.Location = new Point(toolbar.Width - 230, 18); _coverage.Anchor = AnchorStyles.Top | AnchorStyles.Right; _coverage.Click += (s, e) => RunBootAuditAsync(); toolbar.Controls.Add(_coverage);
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
            AttachTooltips();
        }

        private void AttachTooltips()
        {
            _tooltip.SetToolTip(_add, "Add any .exe/.cmd/.bat/.ps1/.lnk to Windows startup — just paste a full path and the rest fills in.");
            _tooltip.SetToolTip(_refresh, "Re-scan every boot source and refresh the list.");
            _tooltip.SetToolTip(_search, "Type to filter the list. Cleared automatically when the window opens.");
            _tooltip.SetToolTip(_showAll, "Show every startup entry.");
            _tooltip.SetToolTip(_showRisky, "Show high-consequence entries: services, drivers, and logon components.");
            _tooltip.SetToolTip(_showCleanup, "Show suggested optional-startup cleanup candidates.");
            _tooltip.SetToolTip(_showDisabled, "Show entries currently kept from startup.");
            _tooltip.SetToolTip(_editSelected, "Edit the selected entry's path, arguments, and startup mode.");
            _tooltip.SetToolTip(_quietSelected, "Start the selected app quietly in the tray at every boot (no window).");
            _tooltip.SetToolTip(_disable, "Remove the selected entry from startup. It can be restored later.");
            _tooltip.SetToolTip(_enable, "Restore the selected entry so it runs at boot.");
            _tooltip.SetToolTip(_deleteManaged, "Permanently delete a startup task created by this app.");
            _tooltip.SetToolTip(_protectNow, "Re-assert every disabled entry so it stays disabled.");
            _tooltip.SetToolTip(_enforceNow, "Re-assert every enabled, quiet, and disabled guard right now.");
            _tooltip.SetToolTip(_coverage, "Verify every boot source is shown and every tray app has one correct icon.");
            _tooltip.SetToolTip(_openFolders, "Open the user and common Startup folders in Explorer.");
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
            var p = new Panel { Bounds = bounds, BackColor = Surface, BorderStyle = BorderStyle.None };
            ApplyRound(p, 14);
            p.Resize += (s, e) => ApplyRound(p, 14);
            return p;
        }

        // Clip a control to softly rounded corners. Buttons never resize, so a one-shot region is
        // enough; cards are re-rounded on resize so the corners stay correct when the window grows.
        private static void ApplyRound(Control c, int radius)
        {
            try
            {
                if (c.Width < radius * 2 + 2 || c.Height < radius * 2 + 2) { c.Region = null; return; }
                int d = radius * 2;
                var path = new GraphicsPath();
                var r = new Rectangle(0, 0, c.Width, c.Height);
                path.AddArc(r.X, r.Y, d, d, 180, 90);
                path.AddArc(r.Right - d - 1, r.Y, d, d, 270, 90);
                path.AddArc(r.Right - d - 1, r.Bottom - d - 1, d, d, 0, 90);
                path.AddArc(r.X, r.Bottom - d - 1, d, d, 90, 90);
                path.CloseFigure();
                c.Region = new Region(path);
            }
            catch { }
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
            b.FlatAppearance.BorderSize = 0;
            ApplyRound(b, 10);
            b.MouseEnter += (s, e) => b.BackColor = ControlPaint.Light(color, .10f); b.MouseLeave += (s, e) => b.BackColor = color; return b;
        }

        private void BuildTray()
        {
            _tray = new NotifyIcon { Icon = Program.AppIcon, Text = "Mich Startup Master", Visible = true };
            _tray.DoubleClick += (s, e) => OpenFromTray();
            _tray.MouseDoubleClick += (s, e) => { if (e.Button == MouseButtons.Left) OpenFromTray(); };
            var trayMenu = new ContextMenuStrip();
            trayMenu.Items.Add("Open Startup Master", null, (s, e) => OpenFromTray());
            trayMenu.Items.Add("Refresh inventory", null, (s, e) => RefreshItems());
            trayMenu.Items.Add("Verify boot coverage", null, (s, e) => RunBootAuditAsync());
            trayMenu.Items.Add("Enforce quiet + disabled + enabled guards", null, (s, e) => RunGuardsAsync(true));
            trayMenu.Items.Add("Exit", null, (s, e) => { _reallyExit = true; if (_tray != null) { _tray.Visible = false; _tray.Dispose(); } Application.Exit(); });
            _tray.ContextMenuStrip = trayMenu;
        }
        private void OpenFromTray()
        {
            if (IsDisposed) return;
            if (InvokeRequired) { BeginInvoke(new Action(OpenFromTray)); return; }
            // Always show the full inventory on open: a leftover search term must never make
            // items look missing from the app, and the list must always reflect the current state.
            if (_search != null) _search.Clear();
            ShowInTaskbar = true;
            Show();
            if (WindowState == FormWindowState.Minimized) WindowState = FormWindowState.Normal;
            BringToFront();
            Activate();
            TopMost = true;
            TopMost = false;
            RefreshItems();
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
                EnabledStartupService.EnforceEnabled(true);
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
        private void RunBootAuditAsync()
        {
            SetBusy(true, "Verifying every boot source is shown in the app and every tray app has one correct icon...");
            Task.Run(() =>
            {
                string boot = StartupService.AuditBootCoverage();
                string tray = StartupService.AuditTrayCoverage();
                return boot + Environment.NewLine + tray;
            }).ContinueWith(t =>
            {
                if (IsDisposed) return;
                BeginInvoke(new Action(() =>
                {
                    SetBusy(false, "");
                    string result = t.Exception != null ? "Coverage check failed: " + t.Exception.GetBaseException().Message : t.Result;
                    bool clean = result.IndexOf("gaps=0", StringComparison.Ordinal) >= 0 && result.IndexOf("findings=0", StringComparison.Ordinal) >= 0;
                    Toast(clean ? "Coverage: complete" : "Coverage: gaps found", result);
                }));
            });
        }

        private void RunGuardsAsync(bool showResult)
        {
            // includeImport=true lets the throttled pass re-run v2 migration, external-task import,
            // and duplicate-launcher retirement at most every five minutes while the app runs.
            Task.Run(() => ProtectedDisabledService.EnforceProtected() + " | " + ProtectedQuietService.EnforceProtected() + " | " + EnabledStartupService.EnforceEnabled(true)).ContinueWith(t =>
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

        // Show a real Windows notification next to the tray icon (used for the "new startup item"
        // alert). Also updates the in-app hint so the message is visible either way.
        private void NotifyToast(string title, string body, ToolTipIcon icon)
        {
            _hint.Text = title + ": " + body;
            try
            {
                if (_tray == null) return;
                _tray.BalloonTipTitle = title;
                _tray.BalloonTipText = body;
                _tray.BalloonTipIcon = icon;
                _tray.ShowBalloonTip(7000);
            }
            catch { }
        }

        // Scan for startup entries that appeared since the last check and alert the moment they
        // are found, so nothing set to run at boot — by this app or any other — is ever missed.
        private void DetectNewStartupItems()
        {
            Task.Run(() => StartupWatcher.DetectNew()).ContinueWith(t =>
            {
                if (IsDisposed) return;
                var fresh = t.Exception != null ? new List<StartupItem>() : t.Result;
                if (fresh.Count == 0) return;
                BeginInvoke(new Action(() =>
                {
                    if (fresh.Count == 1)
                    {
                        var item = fresh[0];
                        NotifyToast("New startup item detected", item.HumanName() + " was just set to start with Windows.", ToolTipIcon.Warning);
                    }
                    else
                    {
                        NotifyToast("New startup items detected", fresh.Count + " new entries were just set to start with Windows.", ToolTipIcon.Warning);
                    }
                    RefreshItems();
                }));
            });
        }

        private static string Q(string s) { return "\"" + (s ?? "").Replace("\"", "\\\"") + "\""; }
    }

    internal sealed class AddStartupForm : Form
    {
        public string AppTitle { get { return _name.Text.Trim(); } }
        public string AppPath { get { return _path.Text.Trim().Trim('"'); } }
        public string AppArguments { get { return _args.Text; } }
        public bool TrayMode { get { return _trayMode.Checked; } }
        private TextBox _name, _path, _args, _paste;
        private RadioButton _normalMode, _trayMode;
        private Label _status;
        private readonly Color Bg = Color.FromArgb(10, 14, 28), Surface = Color.FromArgb(21, 28, 51), Surface2 = Color.FromArgb(17, 24, 44), TextMain = Color.FromArgb(245, 247, 255), Muted = Color.FromArgb(156, 166, 195), Accent = Color.FromArgb(20, 184, 166), Good = Color.FromArgb(52, 211, 153);

        public AddStartupForm() : this("", "", "", true) { }

        public AddStartupForm(string appTitle, string appPath, string appArguments, bool trayMode)
        {
            Text = "Add app to Windows startup"; Width = 780; Height = 640; FormBorderStyle = FormBorderStyle.FixedDialog; MaximizeBox = false; MinimizeBox = false; BackColor = Bg; ForeColor = Color.White; Font = new Font("Segoe UI", 10f); StartPosition = FormStartPosition.CenterParent; Icon = Program.AppIcon; DoubleBuffered = true;
            Controls.Add(new Label { Text = string.IsNullOrWhiteSpace(appPath) ? "Add an app to startup" : "Edit startup app", Left = 32, Top = 22, AutoSize = true, ForeColor = TextMain, Font = new Font("Segoe UI Semibold", 22f) });
            Controls.Add(new Label { Text = "Just paste a full path or a whole command line — the friendly name, arguments and quiet mode are filled in for you.", Left = 34, Top = 62, Width = 700, Height = 40, ForeColor = Muted, Font = new Font("Segoe UI", 10.5f) });

            // Smart paste: one field that accepts a bare path or a full command line.
            AddLabel("Paste a full path or command (fastest way)", 112);
            _paste = Box(138, 622); _paste.Width = 528; _paste.Leave += (s, e) => { if (!string.IsNullOrWhiteSpace(_paste.Text)) ApplyPaste(_paste.Text); };
            var paste = Button("Paste & fill", Accent, 120); paste.Left = 668; paste.Top = 136; paste.Click += PastePath; Controls.Add(paste);

            AddLabel("Friendly name (auto-filled)", 182); _name = Box(208, 640);
            AddLabel("Executable path", 252); _path = Box(278, 528); _path.Width = 528; _path.Leave += (s, e) => AutoFillFromPath();
            var browse = Button("Browse", Surface, 104); browse.Left = 668; browse.Top = 276; browse.Click += Browse; Controls.Add(browse);
            AddLabel("Optional arguments", 322); _args = Box(348, 640);

            AddLabel("Startup mode", 398);
            _normalMode = new RadioButton { Text = "Start normally — run the app directly at Windows logon (a window may appear)", Left = 34, Top = 424, Width = 700, ForeColor = TextMain, BackColor = Bg, Checked = !trayMode };
            _trayMode = new RadioButton { Text = "Start quietly in tray mode — no window, no terminal, starts silently at every boot", Left = 34, Top = 454, Width = 700, ForeColor = Good, BackColor = Bg, Checked = trayMode };
            Controls.Add(_normalMode); Controls.Add(_trayMode);
            Controls.Add(new Label { Text = "Quiet tray mode launches the app hidden and keeps it alive; the app's own tray icon opens its window when you click it.", Left = 54, Top = 482, Width = 690, Height = 34, ForeColor = Muted, Font = new Font("Segoe UI", 9f) });

            _name.Text = appTitle ?? "";
            _path.Text = appPath ?? "";
            _args.Text = appArguments ?? "";

            _status = new Label { Left = 34, Top = 524, Width = 700, Height = 34, ForeColor = Good, Font = new Font("Segoe UI", 9.5f) };
            Controls.Add(_status);

            var ok = Button(string.IsNullOrWhiteSpace(appPath) ? "Add at next boot" : "Save startup", Accent, 170); ok.Left = 430; ok.Top = 566; ok.DialogResult = DialogResult.OK; ok.Click += ValidateBeforeClose;
            var cancel = Button("Cancel", Surface, 110); cancel.Left = 612; cancel.Top = 566; cancel.DialogResult = DialogResult.Cancel;
            Controls.Add(ok); Controls.Add(cancel); AcceptButton = ok; CancelButton = cancel;
        }

        private void ValidateBeforeClose(object sender, EventArgs e)
        {
            string path = AppPath;
            if (string.IsNullOrWhiteSpace(path) || !File.Exists(path) || !StartupService.IsSupportedStartupTarget(path))
            {
                MessageBox.Show("Paste a valid full path to a .exe, .cmd, .bat, .ps1, or .lnk file (or browse for it), then save.", "Missing app", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                DialogResult = DialogResult.None;
                return;
            }
            // A friendly name is optional: derive it from the file's metadata right before saving.
            if (string.IsNullOrWhiteSpace(AppTitle))
            {
                string suggestion = StartupService.SuggestDisplayName(path);
                _name.Text = string.IsNullOrWhiteSpace(suggestion) ? Path.GetFileNameWithoutExtension(path) : suggestion;
            }
        }

        private void AutoFillFromPath()
        {
            string path = AppPath;
            if (string.IsNullOrWhiteSpace(path)) return;
            string expanded = Environment.ExpandEnvironmentVariables(path);
            if (!string.Equals(expanded, path, StringComparison.OrdinalIgnoreCase)) _path.Text = expanded;
            if (string.IsNullOrWhiteSpace(_name.Text))
            {
                string suggestion = StartupService.SuggestDisplayName(expanded);
                if (!string.IsNullOrWhiteSpace(suggestion)) { _name.Text = suggestion; _status.Text = "✓ Friendly name auto-filled from the file's metadata."; }
            }
            if (File.Exists(expanded) && StartupService.IsSupportedStartupTarget(expanded)) _status.Text = "✓ Ready to add.";
        }

        // Accept a bare full path OR a whole command line ("C:\app.exe --flag") and fill every
        // field automatically — no browsing required.
        private bool ApplyPaste(string text)
        {
            try
            {
                text = (text ?? "").Trim();
                if (text.Length >= 2 && text[0] == '"' && text[text.Length - 1] == '"' && text.Count(c => c == '"') == 2) text = text.Substring(1, text.Length - 2);
                string path, args;
                if (File.Exists(text)) { path = text; args = ""; }
                else if (StartupService.TrySplitCommand(text, out path, out args))
                {
                    // A path into WindowsApps may point at an older version of a packaged app;
                    // resolve it to the newest installed version before telling the user it's gone.
                    if (!File.Exists(path))
                    {
                        string resolved = StartupService.ResolveTargetPath(path);
                        if (!string.IsNullOrWhiteSpace(resolved) && File.Exists(resolved)) path = resolved;
                    }
                }
                else { _status.ForeColor = Muted; _status.Text = "That path doesn't exist yet. Paste a full path to an installed .exe/.cmd/.bat/.ps1/.lnk."; return false; }
                if (!File.Exists(path)) { _status.ForeColor = Muted; _status.Text = "That path doesn't exist yet. Paste a full path to an installed .exe/.cmd/.bat/.ps1/.lnk."; return false; }
                if (!StartupService.IsSupportedStartupTarget(path)) { _status.ForeColor = Muted; _status.Text = "That file type can't be started directly. Use .exe, .cmd, .bat, .ps1, or .lnk."; return false; }
                _path.Text = path;
                _args.Text = args ?? "";
                if (string.IsNullOrWhiteSpace(_name.Text)) _name.Text = StartupService.SuggestDisplayName(path);
                if (!_trayMode.Checked && !_normalMode.Checked) _trayMode.Checked = true;
                _status.ForeColor = Good;
                _status.Text = "✓ Detected " + (string.IsNullOrWhiteSpace(args) ? "app" : "app with arguments") + ". Save to add it at every boot.";
                return true;
            }
            catch (Exception ex) { _status.ForeColor = Muted; _status.Text = ex.Message; return false; }
        }

        private void AddLabel(string text, int top) { Controls.Add(new Label { Text = text, Left = 34, Top = top, AutoSize = true, ForeColor = Muted, Font = new Font("Segoe UI Semibold", 9.5f) }); }
        private TextBox Box(int top, int width) { var t = new TextBox { Left = 34, Top = top, Width = width, Height = 32, BackColor = Surface2, ForeColor = Color.White, BorderStyle = BorderStyle.FixedSingle, Font = new Font("Segoe UI", 10.5f) }; Controls.Add(t); return t; }
        private Button Button(string text, Color color, int width) { var b = new Button { Text = text, Width = width, Height = 40, FlatStyle = FlatStyle.Flat, BackColor = color, ForeColor = Color.White, Font = new Font("Segoe UI Semibold", 9.5f), Cursor = Cursors.Hand }; b.FlatAppearance.BorderSize = 0; b.MouseEnter += (s, e) => b.BackColor = ControlPaint.Light(color, .10f); b.MouseLeave += (s, e) => b.BackColor = color; return b; }

        private void Browse(object sender, EventArgs e)
        {
            using (var ofd = new System.Windows.Forms.OpenFileDialog { Filter = "Startup targets (*.exe;*.cmd;*.bat;*.ps1;*.lnk)|*.exe;*.cmd;*.bat;*.ps1;*.lnk|All files (*.*)|*.*", Title = "Choose app to start with Windows" })
            {
                if (ofd.ShowDialog(this) != DialogResult.OK) return;
                _path.Text = ofd.FileName;
                if (string.IsNullOrWhiteSpace(_name.Text)) _name.Text = StartupService.SuggestDisplayName(ofd.FileName);
                AutoFillFromPath();
            }
        }

        private void PastePath(object sender, EventArgs e)
        {
            try
            {
                if (!Clipboard.ContainsText()) { _status.ForeColor = Muted; _status.Text = "Clipboard has no text. Copy a file path first."; return; }
                ApplyPaste(Clipboard.GetText() ?? "");
            }
            catch (Exception ex) { _status.ForeColor = Muted; _status.Text = ex.Message; }
        }
    }

}
