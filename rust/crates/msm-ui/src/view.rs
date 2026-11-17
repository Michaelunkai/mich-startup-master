//! View model: InventoryRow, Apps aggregation, filters, sorting, verify, export TSV.

use msm_core::model::StartupItem;
use msm_core::util::eq_ci;

/// C# `InventoryRow` — one rendered row (application summary or exact route).
#[derive(Clone)]
pub struct InventoryRow {
    pub key: String,
    pub target_identity: String,
    pub display_name: String,
    pub routes: Vec<StartupItem>,
    pub primary: usize, // index into routes
    pub is_app_summary: bool,
}

impl InventoryRow {
    pub fn primary_item(&self) -> &StartupItem {
        &self.routes[self.primary.min(self.routes.len().saturating_sub(1))]
    }

    pub fn is_ambiguous(&self) -> bool {
        self.routes.len() > 1
    }

    pub fn all_enabled(&self) -> bool {
        !self.routes.is_empty() && self.routes.iter().all(|x| display_state(x) == "Enabled")
    }

    pub fn all_disabled(&self) -> bool {
        !self.routes.is_empty() && self.routes.iter().all(|x| display_state(x) == "Disabled")
    }

    pub fn any_enabled(&self) -> bool {
        self.routes.iter().any(|x| display_state(x) == "Enabled")
    }

    pub fn effective_enabled(&self) -> bool {
        if self.is_app_summary { self.any_enabled() } else { self.all_enabled() }
    }

    /// Last user-change timestamp for default "enabled first, newest change first".
    pub fn changed_at(&self) -> u128 {
        self.routes.iter()
            .map(|x| msm_core::denykey::all_keys(x).into_iter().map(|k| msm_state::changed_at::utc_of(&k)).max().unwrap_or(0))
            .max()
            .unwrap_or(0)
    }

    pub fn has_attention(&self) -> bool {
        self.routes.iter().any(|x| x.risk_level() == "Critical" || x.risk_level() == "Review" || x.advice_level() == "Cleanup")
    }

    pub fn has_managed(&self) -> bool {
        self.routes.iter().any(|x| x.is_managed)
    }

    pub fn status_text(&self) -> String {
        if self.effective_enabled() {
            return if self.all_enabled() { "\u{25CF} Enabled".into() } else { "\u{25CF} Enabled \u{00B7} mixed routes".into() };
        }
        if self.all_disabled() {
            return if self.routes.iter().any(route_running_now) { "\u{25CB} Disabled \u{00B7} running now".into() } else { "\u{25CB} Disabled".into() };
        }
        if self.routes.iter().any(|x| x.state_text() == "Drifted") { return "Drifted".into(); }
        "\u{25D0} Mixed".into()
    }

    pub fn aggregate_mode_text(&self) -> String {
        let mut modes: Vec<String> = self.routes.iter().map(mode_text).collect();
        modes.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        if modes.len() == 1 { modes[0].clone() } else { "Mixed".into() }
    }

    pub fn source_text(&self) -> String {
        let mut sources: Vec<&str> = self.routes.iter().map(|x| x.source.as_str()).collect();
        sources.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        if sources.len() == 1 { sources[0].to_string() } else { format!("{} routes", self.routes.len()) }
    }

    pub fn aggregate_impact_text(&self) -> String {
        let highest = self.routes.iter().max_by_key(|x| x.boot_impact_ms()).unwrap_or(&self.routes[0]);
        impact_text(highest)
    }

    pub fn entry_text(&self) -> String {
        if self.routes.len() == 1 { self.primary_item().name.clone() } else { format!("{} registrations", self.routes.len()) }
    }

    /// When this route runs — real trigger times for scheduled tasks
    /// ("Weekly · Mon, Fri · 09:00"), phase text for the other surfaces.
    pub fn schedule_text(&self) -> String {
        let mut schedules: Vec<&str> = self.routes.iter()
            .map(|x| x.schedule.as_str())
            .filter(|s| !s.trim().is_empty())
            .collect();
        schedules.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        if schedules.len() == 1 { return schedules[0].to_string(); }
        if schedules.len() > 1 {
            return format!("{} \u{00B7} +{} more", schedules[0], schedules.len() - 1);
        }
        phase_from_source(&self.primary_item().source).to_string()
    }
}

fn route_running_now(item: &StartupItem) -> bool {
    item.runtime_evidence.to_lowercase().contains("exact target running pid=")
}

/// The truth a user sees: a fresh scan of the owning surface IS the exact
/// state for direct registrations; verified readback refines it; "Unknown"
/// must never render (the user's zero-Unknown law).
pub fn display_state(item: &StartupItem) -> &'static str {
    match item.state_text() {
        "Enabled" => "Enabled",
        "Disabled" => "Disabled",
        "Drifted" => "Drifted",
        _ => if item.enabled { "Enabled" } else { "Disabled" },
    }
}

/// Deny-list lookup for this item (purged/hidden apps never render again).
pub fn is_denied(item: &StartupItem) -> bool {
    !item.id.starts_with("disabled|") && (
        msm_state::denied::contains(&msm_core::denykey::route_key(item))
            || msm_state::denied::contains(&msm_core::denykey::name_key(item))
            || msm_state::denied::contains(&msm_core::denykey::target_key(item))
    )
}

/// Stamp change timestamps after a user mutation (drives the default sort).
pub fn note_change(routes: &[StartupItem]) {
    let mut keys: Vec<String> = Vec::new();
    for route in routes {
        keys.extend(msm_core::denykey::all_keys(route));
    }
    msm_state::changed_at::bump(&keys);
}

/// When a non-task surface runs its registrations.
fn phase_from_source(source: &str) -> &'static str {
    match source {
        "Registry Run" | "Registry RunOnce" | "Registry RunOnceEx" | "Startup Folder"
        | "Startup Approval" | "Startup Command" | "Packaged Startup Task" | "Active Setup"
        | "Group Policy Script" | "Winlogon Autostart" | "Winlogon Notification"
        | "Terminal Server Startup" | "Explorer Startup Extension" | "Legacy v2" => "At log on",
        "Windows Service" | "Service (Trigger Start)" | "System Driver" | "Boot Execute"
        | "LSA Startup Package" | "Network Provider" | "Security Provider" | "AppCert DLLs"
        | "Known DLL" | "Media Codec" => "At boot",
        "Winsock Provider" | "Print Monitor" | "Print Provider" | "Explorer Shell Extension"
        | "Icon Service Library" | "Service Dependency" => "On demand",
        "WMI Event Consumer" => "On event",
        "AppInit DLLs" => "In every GUI app",
        "Image Hijack" => "On launch",
        "URL Search Hook" | "Internet Explorer Add-on" => "In-browser",
        _ => "\u{2014}",
    }
}

fn mode_text(item: &StartupItem) -> String {
    match item.popup_label() {
        "Enabled" => "Window".into(),
        "Disabled" => "Quiet (tray)".into(),
        _ => "Not supported".into(),
    }
}

fn impact_text(item: &StartupItem) -> String {
    match item.boot_impact_label() {
        "High" => "High impact".into(),
        "Medium" => "Medium impact".into(),
        "Low" => "Low impact".into(),
        _ => "None".into(),
    }
}

fn route_severity(item: &StartupItem) -> u8 {
    match item.risk_level() {
        "Critical" => 4,
        "Review" => 3,
        "System" => 2,
        _ => 1,
    }
}

/// C# `SearchText(item)` for row matching.
pub fn search_text(item: &StartupItem) -> String {
    [item.human_name().to_string(), item.name.clone(), item.command.clone(), item.source.clone(),
        item.location.clone(), item.status.clone(), item.advice_reason()]
        .into_iter().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" ")
}

/// C# `MatchesFilter`.
pub fn matches_filter(item: &StartupItem, filter: &str) -> bool {
    match filter {
        "Apps" => is_app_route(item),
        "Enabled" => item.enabled || item.state_text() == "Enabled" || item.state_text() == "Drifted",
        "Disabled" => !item.enabled || item.state_text() == "Disabled",
        _ => true,
    }
}

/// Column sort modes. `Smart` is the default standing order: enabled first,
/// most-recently-changed first (the user's "just-enabled shows first" law).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortMode { Smart, Name, Source, Schedule, Impact, Status }

impl SortMode {
    pub const ALL: [SortMode; 6] = [SortMode::Smart, SortMode::Name, SortMode::Source, SortMode::Schedule, SortMode::Impact, SortMode::Status];
    pub fn label(self) -> &'static str {
        match self { SortMode::Smart => "Recent", SortMode::Name => "Name", SortMode::Source => "Source", SortMode::Schedule => "Schedule", SortMode::Impact => "Impact", SortMode::Status => "Status" }
    }
}

/// C# `IsAppRoute` — exact excluded-source list.
pub fn is_app_route(item: &StartupItem) -> bool {
    if item.id.starts_with("error|") { return false; }
    if msm_core::humans::has_concrete_user_application_target(item) { return true; }
    if item.source == "Windows Service" {
        return item.application_identity.starts_with("installed-product:");
    }
    !matches!(item.source.as_str(),
        "System Driver" | "Winlogon Autostart" | "Winlogon Notification" | "AppInit DLLs"
        | "Active Setup" | "Boot Execute" | "Image Hijack" | "Known DLL" | "Network Provider"
        | "Winsock Provider" | "Print Monitor" | "Print Provider" | "Security Provider"
        | "Terminal Server Startup" | "Icon Service Library" | "URL Search Hook"
        | "Service Dependency" | "Media Codec" | "WMI Event Consumer" | "Group Policy Script"
        | "Explorer Startup Extension" | "Explorer Shell Extension" | "Internet Explorer Add-on"
        | "AppCert DLLs" | "LSA Startup Package" | "Stale Startup Metadata")
}

/// C# `CanonicalTargetIdentity`.
pub fn canonical_target_identity(item: &StartupItem) -> String {
    if item.source == "Startup Approval" {
        return format!("approval:{}", if item.id.is_empty() { item.location.clone() } else { item.id.clone() });
    }
    let mut target = String::new();
    let mut arguments = String::new();
    if item.source == "Startup Folder" && item.command.to_lowercase().ends_with(".lnk") {
        let (t, a) = msm_core::humans::resolve_shortcut(&item.command);
        target = t;
        arguments = a;
    }
    if target.trim().is_empty() {
        if let Some((t, a)) = msm_core::humans::try_decode_tray_payload(&item.command) {
            target = t;
            arguments = a;
        }
    }
    if !target.trim().is_empty() {
        return normalize_target_identity(&target);
    }
    let payloads = msm_core::humans::application_payload_candidates(item);
    if let Some(first) = payloads.first() {
        return normalize_target_identity(first);
    }
    if let Some((t, _)) = msm_core::humans::try_split_command(&item.command) {
        if !t.trim().is_empty() {
            return normalize_target_identity(&t);
        }
    }
    let _ = arguments;
    let raw: String = item.command.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    if raw.is_empty() {
        format!("route:{}", if item.id.is_empty() { format!("{}|{}|{}", item.source, item.location, item.name) } else { item.id.clone() })
    } else {
        format!("raw:{}", raw)
    }
}

pub fn normalize_target_identity(target: &str) -> String {
    let value = msm_core::util::expand_env(target.trim().trim_matches('"')).replace('/', "\\");
    let value = if std::path::Path::new(&value).is_absolute() { msm_core::paths::canonical(&value) } else { value };
    format!("path:{}", value.trim_end_matches('\\').to_lowercase())
}

/// C# `ApplicationGroupIdentity`.
pub fn application_group_identity(item: &StartupItem) -> String {
    if !item.application_identity.trim().is_empty() {
        item.application_identity.clone()
    } else {
        canonical_target_identity(item)
    }
}

/// C# `BuildVisibleRows` — default standing order (Smart sort).
pub fn build_visible_rows(items: &[StartupItem], filter: &str, query: &str) -> Vec<InventoryRow> {
    build_visible_rows_sorted(items, filter, query, SortMode::Smart)
}

/// `BuildVisibleRows` with an explicit column sort. Purged/hidden apps never
/// render (deny-list); the Hidden view lists exactly those, ungrouped.
pub fn build_visible_rows_sorted(items: &[StartupItem], filter: &str, query: &str, sort: SortMode) -> Vec<InventoryRow> {
    let filtered = query.trim().to_lowercase();
    let hidden_view = filter == "Hidden";
    let visible: Vec<&StartupItem> = items.iter()
        .filter(|i| if hidden_view { is_denied(i) || i.id.starts_with("error|") } else { !is_denied(i) })
        .filter(|i| hidden_view || matches_filter(i, filter))
        .collect();
    let mut rows: Vec<InventoryRow> = Vec::new();
    if filter == "Apps" && !hidden_view {
        // Stable grouping by application identity.
        let mut groups: Vec<(String, Vec<StartupItem>)> = Vec::new();
        for item in visible {
            let identity = application_group_identity(item);
            match groups.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(&identity)) {
                Some((_, group)) => group.push(item.clone()),
                None => groups.push((identity, vec![item.clone()])),
            }
        }
        for (identity, routes) in groups {
            if !filtered.is_empty() && !routes.iter().any(|x| search_text(x).to_lowercase().contains(&filtered)) {
                continue;
            }
            let primary = pick_primary(&routes);
            rows.push(InventoryRow {
                key: format!("app|{}", identity),
                target_identity: identity,
                display_name: String::new(),
                routes,
                primary,
                is_app_summary: true,
            });
        }
    } else {
        for item in visible {
            if !filtered.is_empty() && !search_text(item).to_lowercase().contains(&filtered) {
                continue;
            }
            let identity = canonical_target_identity(item);
            rows.push(InventoryRow {
                key: format!("route|{}", if item.id.is_empty() { identity.clone() } else { item.id.clone() }),
                target_identity: identity,
                display_name: String::new(),
                routes: vec![item.clone()],
                primary: 0,
                is_app_summary: false,
            });
        }
    }
    // Disambiguate repeated display names: "Name  ·  parent\file".
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for row in &rows {
        *counts.entry(row.routes[row.primary.min(row.routes.len() - 1)].human_name().to_lowercase()).or_insert(0) += 1;
    }
    for row in &mut rows {
        let human = row.routes[row.primary.min(row.routes.len() - 1)].human_name().to_string();
        row.display_name = if counts.get(&human.to_lowercase()).copied().unwrap_or(0) > 1 {
            let primary_item = &row.routes[row.primary.min(row.routes.len() - 1)];
            format!("{}  \u{00B7}  {}", human, readable_disambiguator(&row.target_identity, primary_item))
        } else {
            human
        };
    }
    let first_seen_of = |row: &InventoryRow| row.routes.iter().map(msm_state::first_seen::utc_of).max().unwrap_or(0);
    let impact_rank = |row: &InventoryRow| row.routes.iter().map(|x| x.boot_impact_ms()).max().unwrap_or(0);
    match sort {
        SortMode::Name => rows.sort_by(|a, b| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase())
            .then_with(|| a.target_identity.to_lowercase().cmp(&b.target_identity.to_lowercase()))),
        SortMode::Source => rows.sort_by(|a, b| a.source_text().to_lowercase().cmp(&b.source_text().to_lowercase())
            .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))),
        SortMode::Schedule => rows.sort_by(|a, b| a.schedule_text().cmp(&b.schedule_text())
            .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))),
        SortMode::Impact => rows.sort_by(|a, b| impact_rank(b).cmp(&impact_rank(a))
            .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))),
        SortMode::Status => rows.sort_by(|a, b| (b.effective_enabled() as u8).cmp(&(a.effective_enabled() as u8))
            .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))),
        SortMode::Smart => rows.sort_by(|a, b| {
            // Newest activity first: whatever was added, enabled or disabled last is row 1.
            b.changed_at().max(first_seen_of(b)).cmp(&a.changed_at().max(first_seen_of(a)))
                .then_with(|| (b.effective_enabled() as u8).cmp(&(a.effective_enabled() as u8)))
                .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))
                .then_with(|| a.target_identity.to_lowercase().cmp(&b.target_identity.to_lowercase()))
        }),
    }
    rows
}

fn pick_primary(routes: &[StartupItem]) -> usize {
    let mut best = 0usize;
    let mut best_key = (0u8, false, String::new());
    for (index, item) in routes.iter().enumerate() {
        let key = (route_severity(item), item.is_managed, item.human_name().to_lowercase());
        if key > best_key {
            best_key = key;
            best = index;
        }
    }
    best
}

/// Human-readable tail for duplicate names. Internal identities (`approval:<base64 id>`,
/// `route:<id>`, `raw:<cmd>`) are never shown: fall back to the source and the last
/// segment of the registry/file location, which a person can recognise.
pub fn readable_disambiguator(identity: &str, item: &StartupItem) -> String {
    let lower = identity.to_lowercase();
    if lower.starts_with("approval:") || lower.starts_with("route:") || lower.starts_with("raw:") || identity.contains('|') {
        let loc = item.location.trim();
        let tail = loc.rsplit(|c| c == '\\' || c == '/').next().unwrap_or(loc);
        let tail = if tail.is_empty() { loc } else { tail };
        let tail: String = tail.chars().take(48).collect();
        return if tail.is_empty() { item.source.clone() } else { format!("{} \u{2014} {}", item.source, tail) };
    }
    target_disambiguator(identity)
}

pub fn target_disambiguator(identity: &str) -> String {
    let mut value = identity.to_string();
    if let Some(stripped) = value.strip_prefix("path:") {
        value = stripped.to_string();
    } else if let Some(stripped) = value.strip_prefix("installed-product:") {
        value = stripped.to_string();
        if let Some(index) = value.to_lowercase().find("|registration:") {
            value = value[..index].to_string();
        }
    }
    let file = value.rsplit('\\').next().unwrap_or("").to_string();
    let parent = value.rsplit_once('\\').map(|(dir, _)| dir.rsplit('\\').next().unwrap_or("").to_string()).unwrap_or_default();
    if !parent.is_empty() && !file.is_empty() {
        format!("{}\\{}", parent, file)
    } else if !file.is_empty() {
        file
    } else {
        value
    }
}

/// C# `IsCoveredStartupCommandMirror` + `BulkStateCandidates` + `CanBulkSetEnabled`.
pub fn is_covered_startup_command_mirror(route: &StartupItem, group: &[StartupItem]) -> bool {
    if route.source != "Startup Command" { return false; }
    let mirror_command = msm_core::dedupe::norm_boot_key(&route.command);
    if mirror_command.len() <= 8 { return false; }
    group.iter().any(|other| {
        !(other.source == "Startup Command") && route_command_covers_mirror(other, &mirror_command)
    })
}

fn route_command_covers_mirror(candidate: &StartupItem, mirror_command: &str) -> bool {
    let command_key = msm_core::dedupe::norm_boot_key(&candidate.command);
    if command_key.len() > 8 && (command_key == mirror_command || (command_key.len() >= mirror_command.len() && command_key.starts_with(mirror_command))) {
        return true;
    }
    candidate.application_targets.iter().any(|target| {
        let target_key = msm_core::dedupe::norm_boot_key(target);
        target_key.len() > 8 && (target_key == mirror_command || mirror_command.starts_with(&target_key))
    })
}

pub fn bulk_state_candidates(row: &InventoryRow, enabled_state: bool) -> Vec<StartupItem> {
    row.routes.iter()
        .filter(|route| route.enabled != enabled_state
            && !eq_ci(&route.source, "Scheduled Task (Any Trigger)")
            && !eq_ci(&route.source, "Service (Trigger Start)")
            && !is_covered_startup_command_mirror(route, &row.routes))
        .cloned()
        .collect()
}

pub fn can_bulk_set_enabled(row: &InventoryRow, enabled_state: bool) -> bool {
    if !row.is_ambiguous() { return false; }
    let candidates = bulk_state_candidates(row, enabled_state);
    !candidates.is_empty()
        && candidates.iter().all(|route| route.can_disable && route.external_authority.trim().is_empty())
}

/// C# `BuildStartupExportTsv` — exact 35-column header and row values.
pub fn build_startup_export_tsv(items: &[StartupItem]) -> String {
    let mut rows: Vec<&StartupItem> = items.iter().collect();
    rows.sort_by(|a, b| a.human_name().to_lowercase().cmp(&b.human_name().to_lowercase())
        .then_with(|| a.id.to_lowercase().cmp(&b.id.to_lowercase())));
    let mut output = String::from("Startup ID\tApplication\tRegistration name\tSource\tScope\tConfigured enabled\tVerified state\tStatus\tEvidence reason\tPresentation state\tPresentation reason\tStartup mode\tPopup enabled\tRisk\tCan disable\tManaged by Startup Master\tLocation\tLaunch command\tApplication identity\tApplication root\tApplication identity reason\tApplication targets\tRuntime evidence\tMutation capability\tMutation reason\tRequires expert confirmation\tRequires elevation\tRequires reboot\tExternal authority\tApproval root\tApproval path\tApproval name\tApproval data hex\tApproval registry view\tRegistry view");
    for item in rows {
        let tsv = |value: &str| value.replace('\t', " ").replace("\r\n", "\\n").replace('\r', "\\n").replace('\n', "\\n");
        let boolean = |value: bool| if value { "true".to_string() } else { "false".to_string() };
        let bytes_hex = |data: &Option<Vec<u8>>| data.as_ref()
            .map(|d| d.iter().map(|b| format!("{:02X}", b)).collect::<String>())
            .unwrap_or_default();
        let fields = [
            tsv(&item.id), tsv(item.human_name()), tsv(&item.name), tsv(&item.source), tsv(&item.scope),
            boolean(item.enabled), tsv(&item.verified_state), tsv(&item.status), tsv(&item.evidence_reason),
            tsv(&item.presentation_state), tsv(&item.presentation_reason), item.popup_label().to_string(),
            boolean(item.popup_enabled()), item.risk_level().to_string(), boolean(item.can_disable),
            boolean(item.is_managed), tsv(&item.location), tsv(&item.command), tsv(&item.application_identity),
            tsv(&item.application_root), tsv(&item.application_identity_reason),
            tsv(&item.application_targets.join("; ")), tsv(&item.runtime_evidence),
            tsv(&item.mutation_capability), tsv(&item.mutation_reason), boolean(item.requires_expert_confirmation),
            boolean(item.requires_elevation), boolean(item.requires_reboot), tsv(&item.external_authority),
            tsv(&item.approval_root), tsv(&item.approval_path), tsv(&item.approval_name),
            bytes_hex(&item.approval_data), tsv(&item.approval_registry_view), tsv(&item.registry_view),
        ];
        output.push_str("\r\n");
        output.push_str(&fields.join("\t"));
    }
    output
}
