//! The startup item row model — a faithful port of the C# `StartupItem`.

use crate::util::{eq_ci, is_elevated};

/// One startup registration as collected from one Windows surface.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct StartupItem {
    pub id: String,
    pub name: String,
    pub source: String,
    pub scope: String,
    pub command: String,
    pub location: String,
    pub app_name: String,
    /// Stable application ownership from an installed-product path registration,
    /// a Windows app package family, a script entry point, or an exact executable.
    pub application_identity: String,
    pub application_root: String,
    pub application_identity_reason: String,
    /// Exact executable candidates discovered behind a launcher or script
    /// (in-memory projection only; mutations always target the original id).
    pub application_targets: Vec<String>,
    pub runtime_evidence: String,
    pub enabled: bool,
    /// Configuration is not proof; rebuilt during each read-only scan.
    pub verified_state: String,
    pub evidence_reason: String,
    pub presentation_state: String,
    pub presentation_reason: String,
    pub can_disable: bool,
    pub is_managed: bool,
    pub status: String,
    /// Human schedule text: "Weekly · Mon, Fri · 09:00", "At log on", ...
    pub schedule: String,
    // Explorer StartupApproved overlay on a real Run/folder registration.
    pub approval_root: String,
    pub approval_path: String,
    pub approval_name: String,
    pub approval_data: Option<Vec<u8>>,
    pub approval_registry_view: String,
    /// Explicit registry view for view-sensitive SOFTWARE registrations.
    pub registry_view: String,
    pub mutation_capability: String,
    pub mutation_reason: String,
    pub requires_expert_confirmation: bool,
    pub requires_elevation: bool,
    pub requires_reboot: bool,
    pub external_authority: String,
}

impl StartupItem {
    pub fn new() -> Self {
        StartupItem {
            verified_state: "Unknown".into(),
            evidence_reason: "Exact registration and target readback has not been verified".into(),
            presentation_state: "Needs verification".into(),
            presentation_reason: "Startup window/tray presentation has not been observed".into(),
            ..Default::default()
        }
    }

    pub fn human_name(&self) -> &str {
        if !self.app_name.trim().is_empty() {
            return &self.app_name;
        }
        if self.name.trim().is_empty() {
            "(startup item)"
        } else {
            &self.name
        }
    }

    pub fn state_text(&self) -> &str {
        if self.id.starts_with("preview|") {
            if self.enabled { "Enabled" } else { "Disabled" }
        } else {
            &self.verified_state
        }
    }

    /// Sources that have no application presentation contract (popup N/A list).
    const NA_SOURCES: [&'static str; 26] = [
        "Windows Service", "System Driver", "Winlogon Autostart", "Winlogon Notification",
        "AppInit DLLs", "AppCert DLLs", "LSA Startup Package", "Active Setup", "Boot Execute",
        "Image Hijack", "Known DLL", "Network Provider", "Winsock Provider", "Print Monitor",
        "Print Provider", "Security Provider", "Terminal Server Startup", "Icon Service Library",
        "URL Search Hook", "Service Dependency", "Media Codec", "WMI Event Consumer",
        "Group Policy Script", "Explorer Startup Extension", "Explorer Shell Extension",
        "Internet Explorer Add-on",
    ];

    pub fn popup_label(&self) -> &'static str {
        if self.id.starts_with("error|") {
            return "N/A";
        }
        if Self::NA_SOURCES.iter().any(|s| eq_ci(&self.source, s)) {
            return "N/A";
        }
        if Self::NA_SOURCES.iter().any(|s| eq_ci(&self.source, s)) || self.source == "Startup Approval" || self.source == "Stale Startup Metadata" {
            return "N/A";
        }
        if !self.enabled {
            return "Disabled";
        }
        if crate::scan::is_quiet_launch(&self.command, &self.location) {
            "Disabled"
        } else {
            "Enabled"
        }
    }

    pub fn popup_enabled(&self) -> bool {
        self.popup_label() == "Enabled"
    }

    pub fn risk_level(&self) -> &'static str {
        let c = self.command.to_lowercase();
        let n = self.name.to_lowercase();
        let s = self.source.to_lowercase();
        let st = self.status.to_lowercase();
        if s == "system driver" || s == "winlogon autostart" || s == "appinit dlls" || s == "boot execute" || s == "lsa startup package" {
            return "Critical";
        }
        if s == "windows service" && is_security_or_core_service(&(n.clone() + " " + &c + " " + &st)) {
            return "Critical";
        }
        if s == "scheduled task" && st.contains("boot") && self.location.to_lowercase().starts_with("\\microsoft\\windows\\") {
            return "Critical";
        }
        if c.contains("temp") || c.contains("appdata\\local\\temp") || c.contains("powershell") || c.contains("cmd.exe")
            || c.contains("wscript.exe") || c.contains("cscript.exe") {
            return "Review";
        }
        if self.scope == "Machine" {
            return "System";
        }
        "Normal"
    }

    pub fn risk_label(&self) -> &'static str {
        match self.risk_level() {
            "Critical" => "High impact",
            "Review" => "Review",
            "System" => "System",
            _ => "Normal",
        }
    }

    pub fn risk_reason(&self) -> &'static str {
        let level = self.risk_level();
        if level == "Critical" {
            return match self.source.as_str() {
                "System Driver" => "Boot/system driver. Disabling can break hardware, security, or Windows startup.",
                "Winlogon Autostart" => "Windows logon component. Wrong changes can break sign-in.",
                "AppInit DLLs" => "DLL injection startup point. Changes can affect many desktop apps.",
                "Windows Service" => "Security or core Windows service. Disabling can reduce protection or break system behavior.",
                _ => "Microsoft boot/startup infrastructure task.",
            };
        }
        if level == "Review" {
            return "Script or shell-based startup command. Inspect before trusting.";
        }
        if level == "System" {
            return "Machine-wide startup item.";
        }
        "Normal user/app startup item."
    }

    pub fn advice_level(&self) -> &'static str {
        if !self.enabled || self.risk_level() == "Critical" {
            return "Keep";
        }
        let haystack = format!("{} {} {} {} {}", self.name, self.app_name, self.command, self.location, self.status).to_lowercase();
        let optional_updater = regex::Regex::new(
            r"\b(updater|update task|update check|updatecurrentuser|updatecurrentmachine|--wake|/wake)\b").unwrap()
            .is_match(&haystack);
        let tray_helper = regex::Regex::new(r"\b(tray icon|tray_icon|trayicon|helper tray|notification icon)\b").unwrap().is_match(&haystack);
        let telemetry = regex::Regex::new(r"\b(telemetry|crash reporter|crashreporter|usage reporter|feedback uploader)\b").unwrap().is_match(&haystack);
        let installer_watcher = regex::Regex::new(r"\b(installer dialog watchdog|auto.?install guardian|visible downloads live sync|popup rescue)\b").unwrap().is_match(&haystack);
        if optional_updater || tray_helper || telemetry || installer_watcher {
            "Cleanup"
        } else {
            "Keep"
        }
    }

    pub fn advice_label(&self) -> &'static str {
        if self.advice_level() == "Cleanup" { "Review" } else { "No signal" }
    }

    pub fn advice_reason(&self) -> String {
        if self.advice_level() != "Cleanup" {
            return "No optional-startup pattern was detected. This is not a safety or trust verdict.".to_string();
        }
        let haystack = format!("{} {} {} {} {}", self.name, self.app_name, self.command, self.location, self.status).to_lowercase();
        if haystack.contains("telemetry") || haystack.contains("crash reporter") || haystack.contains("crashreporter") {
            return "Telemetry or crash-reporting pattern. Review the app's purpose before changing it.".into();
        }
        if haystack.contains("tray icon") || haystack.contains("tray_icon") || haystack.contains("trayicon") {
            return "Tray-helper pattern. Review whether you rely on its notifications or controls.".into();
        }
        if haystack.contains("installer dialog watchdog") || haystack.contains("auto-install guardian") || haystack.contains("popup rescue") {
            return "Installer/watchdog pattern. Review its workflow before changing startup state.".into();
        }
        "Updater/checker pattern. Review the app's update behavior before changing startup state.".into()
    }
}

fn is_security_or_core_service(text: &str) -> bool {
    const NEEDLES: [&str; 16] = ["defender", "windefend", "security", "firewall", "antivirus",
        "anti-malware", "antimalware", "malwarebytes", "mbam", "vpn", "credential", "crypt",
        "event log", "rpc", "plugplay", "winmgmt"];
    NEEDLES.iter().any(|n| text.to_lowercase().contains(n))
}

/// `RegistrationVerdict` — exact truth verdict from readback evidence.
pub fn registration_verdict(readback_available: bool, exact_action: bool, target_readable: bool,
                            intent_matches: bool, scanned_enabled: bool, readback_enabled: bool) -> &'static str {
    if !readback_available { return "Unknown"; }
    if !exact_action || !target_readable || !intent_matches || scanned_enabled != readback_enabled { return "Drifted"; }
    if readback_enabled { "Enabled" } else { "Disabled" }
}

/// A scan warning row for a provider that failed visibly.
pub fn error_item(source: &str, scope: &str, location: &str, message: &str) -> StartupItem {
    StartupItem {
        id: format!("error|{}|{}", source, location),
        name: "Scan warning".into(),
        source: source.into(),
        scope: scope.into(),
        location: location.into(),
        command: message.into(),
        enabled: false,
        can_disable: false,
        status: "Read failed".into(),
        ..StartupItem::new()
    }
}

/// Startup items that represent a direct, configured startup registration
/// (fresh read of the owning surface verifies Enabled/Disabled exactly).
pub fn is_direct_configured_startup_source(source: &str) -> bool {
    matches!(source,
        "Registry Run" | "Registry RunOnce" | "Registry RunOnceEx" | "Registry RunServices"
        | "Policy Run" | "Legacy Windows Run" | "User Logon Script" | "Startup Folder"
        | "Windows Service" | "System Driver" | "Winlogon Autostart" | "Boot Execute"
        | "LSA Startup Package" | "Group Policy Script")
}
