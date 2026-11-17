//! Packaged (MSIX/Store) startup tasks: manifest declarations + AppModel state authority.

use crate::ids;
use crate::model::{error_item, StartupItem};
use crate::reg::*;
use crate::util::eq_ci;

pub struct PackageManifestLocation {
    pub manifest_path: String,
    pub package_full_name: String,
    pub package_family_name: String,
    pub scope: String,
    pub display_name: String,
}

pub fn package_family_from_full_name(full: &str) -> String {
    let first = full.find('_');
    let last = full.rfind('_');
    match (first, last) {
        (Some(f), Some(l)) if f > 0 && l > f => {
            let publisher = &full[l + 1..];
            // A package FAMILY name is `<Name>_<PublisherId>` (underscore included): it is the exact
            // key under AppModel\SystemAppData that holds the StartupTask State Windows honours.
            if publisher.trim().is_empty() { full.to_string() } else { format!("{}_{}", &full[..f], publisher) }
        }
        _ => full.to_string(),
    }
}

/// C# `PackagedStartupTaskStatePath`.
pub fn packaged_startup_task_state_path(registration_name: &str) -> Option<String> {
    let name = registration_name;
    let bang = name.rfind('!')?;
    if bang == 0 || bang == name.len() - 1 {
        return None;
    }
    Some(format!(r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData\{}\{}", &name[..bang], &name[bang + 1..]))
}

pub fn read_packaged_startup_task_state(registration_name: &str) -> Option<i64> {
    let path = packaged_startup_task_state_path(registration_name)?;
    let key = open_subkey(RegRoot::CurrentUser, &path, RegView::Default)?;
    let (_, value) = key.value("State")?;
    match value {
        RegValue::DWord(v) => Some(v as i64),
        RegValue::QWord(v) => Some(v as i64),
        RegValue::Str(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// C# `ResolvePackagedStartupTaskAuthority` — State 3/4 are policy-controlled.
pub fn resolve_packaged_startup_task_authority(item: &mut StartupItem, state: Option<i64>) {
    if let Some(state) = state {
        if state == 3 || state == 4 {
            item.can_disable = false;
            item.mutation_capability = "ExternalAuthority".into();
            item.mutation_reason = format!("Packaged startup task State={} is policy-controlled; only the Windows policy authority can change it", state);
            item.external_authority = "Windows packaged startup policy".into();
            item.status = format!("{}; AppModel State={} (policy-controlled)", item.status, state);
            return;
        }
        item.enabled = state == 2;
    }
    item.can_disable = true;
    item.mutation_capability = "AppModelStartupTaskState".into();
    item.mutation_reason = "Toggle the AppModel SystemAppData State value (2=enabled, 1=disabled by user) — the same value Windows Settings writes".into();
    item.status = format!("{}; AppModel State={}", item.status,
        state.map(|s| s.to_string()).unwrap_or_else(|| "unset (manifest default)".into()));
}

/// C# `SetPackagedStartupTaskState` — write the AppModel State authority.
pub fn set_packaged_startup_task_state(item: &mut StartupItem, enabled: bool) -> Result<(), String> {
    let path = packaged_startup_task_state_path(&item.name)
        .ok_or_else(|| "Packaged startup task identity is incomplete; cannot resolve its State key".to_string())?;
    let state: i64 = if enabled { 2 } else { 1 };
    let Some(key) = create_subkey(RegRoot::CurrentUser, &path, RegView::Default) else {
        return Err(format!("Could not open the AppModel startup task state key: {}", path));
    };
    if !key.set_value("State", RegKind::DWord, &RegValue::DWord(state as u32)) {
        return Err("Could not write the AppModel startup task State".into());
    }
    match key.value("State") {
        Some((_, RegValue::DWord(v))) if v as i64 == state => {}
        _ => return Err("Packaged startup task state did not verify after writing".into()),
    }
    item.enabled = enabled;
    Ok(())
}

/// C# `EnumeratePackageManifestLocations` — repository walk (direct manifest open).
pub fn enumerate_package_manifest_locations(errors: Option<&mut Vec<StartupItem>>, independent: bool) -> Vec<PackageManifestLocation> {
    let mut result = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let current_repository = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    let mut scan_repository = |root: RegRoot, repository_path: &str, scope: &str, prefix: Option<&str>,
                                result: &mut Vec<PackageManifestLocation>, seen: &mut Vec<String>,
                                errors: &mut Option<&mut Vec<StartupItem>>| {
        let full_path = match prefix {
            Some(p) => format!("{}\\{}", p, repository_path),
            None => repository_path.to_string(),
        };
        let Some(packages) = open_subkey(root, &full_path, RegView::Default) else { return };
        for package_full_name in packages.subkey_names() {
            let Some(package) = open_subkey(root, &format!("{}\\{}", full_path, package_full_name), RegView::Default) else { continue };
            let mut package_root = package.value("PackageRootFolder").map(|(_, v)| v.display()).unwrap_or_default();
            if package_root.trim().is_empty() {
                package_root = package.value("Path").map(|(_, v)| v.display()).unwrap_or_default();
            }
            package_root = crate::util::expand_env(&package_root);
            if package_root.trim().is_empty() {
                let program_files = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
                package_root = format!("{}\\WindowsApps\\{}", program_files, package_full_name);
            }
            let manifest = format!("{}\\AppxManifest.xml", package_root.trim_end_matches('\\'));
            let mut family = package.value("PackageFamilyName").map(|(_, v)| v.display()).unwrap_or_default();
            if family.trim().is_empty() {
                family = package_family_from_full_name(&package_full_name);
            }
            let display_name = package.value("DisplayName").map(|(_, v)| v.display()).unwrap_or_default();
            let identity = format!("{}|{}", scope, manifest);
            if !seen.iter().any(|s| s.eq_ignore_ascii_case(&identity)) {
                seen.push(identity);
                result.push(PackageManifestLocation { manifest_path: manifest, package_full_name, package_family_name: family, scope: scope.into(), display_name });
            }
        }
    };
    let mut err_sink: Option<&mut Vec<StartupItem>> = errors;
    scan_repository(RegRoot::CurrentUser, current_repository, "User", None, &mut result, &mut seen, &mut err_sink);
    scan_repository(RegRoot::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages", "Machine", None, &mut result, &mut seen, &mut err_sink);
    for (sid, scope) in crate::hive::loaded_user_hives() {
        scan_repository(RegRoot::Users, current_repository, &scope, Some(&sid), &mut result, &mut seen, &mut err_sink);
    }
    let _ = independent;
    result
}

/// C# `ReadPackageManifestText` — direct open, no existence probe.
pub fn read_package_manifest_text(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// C# `AddPackagedStartupTasks` / `AuditPackagedStartupTasks`.
pub fn collect_packaged_startup_tasks(items: &mut Vec<StartupItem>, independent: bool) {
    let timing = std::env::var("MSM_STAGE_TIMING").is_ok();
    let started = std::time::Instant::now();
    let locations = enumerate_package_manifest_locations(Some(items), independent);
    if timing {
        eprintln!("[stage] packaged-enumerate {:?} ({} locations)", started.elapsed(), locations.len());
    }
    let mut index = 0usize;
    for location in locations {
        index += 1;
        if timing && index % 50 == 0 {
            eprintln!("[stage] packaged-progress {} at {:?}", index, started.elapsed());
        }
        let read_started = std::time::Instant::now();
        if timing { eprintln!("[stage] manifest-read {}", location.manifest_path); }
        let Some(xml) = read_package_manifest_text(&location.manifest_path) else { continue };
        if timing { eprintln!("[stage] manifest-read-done {} bytes {}", xml.len(), location.manifest_path); }
        let parse_started = std::time::Instant::now();
        if timing && read_started.elapsed().as_millis() > 100 {
            eprintln!("[stage] slow-manifest-read {:?} {}", read_started.elapsed(), location.manifest_path);
        }
        match parse_packaged_startup_manifest(&xml, &location) {
            Ok(mut rows) => {
                for row in rows.iter_mut() {
                    let state = read_packaged_startup_task_state(&row.name);
                    resolve_packaged_startup_task_authority(row, state);
                }
                items.extend(rows);
            }
            Err(e) => items.push(error_item(&format!("{}Packaged Startup Task", if independent { "Independent " } else { "" }), &location.scope, &location.manifest_path, &e)),
        }
        if timing && parse_started.elapsed().as_millis() > 100 {
            eprintln!("[stage] slow-manifest-parse {:?} {}", parse_started.elapsed(), location.manifest_path);
        }
    }
    if timing {
        eprintln!("[stage] packaged-total {:?} ({} manifests)", started.elapsed(), index);
    }
}

/// Minimal namespace-agnostic XML reader for AppxManifest startupTask extensions.
pub fn parse_packaged_startup_manifest(xml: &str, location: &PackageManifestLocation) -> Result<Vec<StartupItem>, String> {
    let doc = xml_doc(xml)?;
    let package = doc.root.ok_or("Package manifest root is missing")?;
    if !eq_ci(&package.name.local_name, "Package") {
        return Err("Package manifest root is missing".into());
    }
    let mut package_name = String::new();
    if let Some(identity) = find_child(&package, "Identity") {
        package_name = identity.attributes.iter().find(|a| eq_ci(&a.name.local_name, "Name")).map(|a| a.value.clone()).unwrap_or_default();
    }
    let mut display_name = String::new();
    if let Some(properties) = find_child(&package, "Properties") {
        if let Some(display) = find_child(properties, "DisplayName") {
            display_name = display.text.trim().to_string();
        }
    }
    let family = if location.package_family_name.trim().is_empty() {
        package_family_from_full_name(&location.package_full_name)
    } else {
        location.package_family_name.clone()
    };
    let mut items = Vec::new();
    for extension in find_all(&package, "Extension") {
        let category = extension.attributes.iter().find(|a| eq_ci(&a.name.local_name, "Category")).map(|a| a.value.clone()).unwrap_or_default();
        if !category.to_lowercase().ends_with(".startuptask") && !eq_ci(&category, "windows.startupTask") {
            continue;
        }
        let executable = extension.attributes.iter().find(|a| eq_ci(&a.name.local_name, "Executable")).map(|a| a.value.clone()).unwrap_or_default();
        let entry_point = extension.attributes.iter().find(|a| eq_ci(&a.name.local_name, "EntryPoint")).map(|a| a.value.clone()).unwrap_or_default();
        for task in find_all(&extension, "StartupTask") {
            let task_id = task.attributes.iter().find(|a| eq_ci(&a.name.local_name, "TaskId")).map(|a| a.value.clone()).unwrap_or_default();
            if task_id.trim().is_empty() {
                continue;
            }
            let task_display = task.attributes.iter().find(|a| eq_ci(&a.name.local_name, "DisplayName")).map(|a| a.value.clone()).unwrap_or_default();
            let manifest_enabled = task.attributes.iter().find(|a| eq_ci(&a.name.local_name, "Enabled"))
                .and_then(|a| a.value.parse::<bool>().ok()).unwrap_or(false);
            let registration_name = if family.trim().is_empty() { task_id.clone() } else { format!("{}!{}", family, task_id) };
            let mut command = executable.clone();
            if !command.trim().is_empty() && !std::path::Path::new(&command).is_absolute() {
                let dir = location.manifest_path.rfind('\\').map(|i| &location.manifest_path[..i]).unwrap_or("");
                command = format!("{}\\{}", dir, command);
            }
            if command.trim().is_empty() {
                command = format!("Package activation {}{}", registration_name, if entry_point.trim().is_empty() { String::new() } else { format!(" ({})", entry_point) });
            }
            let mut human = task_display.clone();
            if human.is_empty() || human.starts_with("ms-resource:") {
                human = display_name.clone();
            }
            if human.is_empty() || human.starts_with("ms-resource:") {
                human = if package_name.is_empty() { task_id.clone() } else { package_name.clone() };
            }
            items.push(StartupItem {
                id: ids::packaged_id(&location.scope, &location.manifest_path, &registration_name),
                name: registration_name,
                app_name: crate::humans::clean_name(&human),
                source: "Packaged Startup Task".into(),
                scope: location.scope.clone(),
                command,
                location: format!("{} :: {}", location.manifest_path, task_id),
                enabled: manifest_enabled,
                can_disable: false,
                status: format!("Declared by package manifest; taskId={}; manifestEnabled={}", task_id, manifest_enabled),
                ..StartupItem::new()
            });
        }
    }
    Ok(items)
}

// ---- tiny namespace-agnostic XML model (shared with task XML parsing) ----

#[derive(Debug, Clone)]
pub struct XmlName {
    pub local_name: String,
}

#[derive(Debug, Clone)]
pub struct XmlAttribute {
    pub name: XmlName,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct XmlNode {
    pub name: XmlName,
    pub attributes: Vec<XmlAttribute>,
    pub children: Vec<XmlNode>,
    pub text: String,
}

pub struct XmlDocument {
    pub root: Option<XmlNode>,
}

pub fn xml_doc(xml: &str) -> Result<XmlDocument, String> {
    let trimmed = xml.trim_start();
    if trimmed.starts_with("<!") || trimmed.contains("<!DOCTYPE") {
        return Err("DTD is prohibited in this document".into());
    }
    let mut parser = Parser { chars: xml.as_bytes(), pos: 0 };
    let mut root = None;
    while let Some(node) = parser.parse_node(0)? {
        if !node.name.local_name.starts_with('#') {
            root = Some(node);
            break;
        }
    }
    Ok(XmlDocument { root })
}

struct Parser<'a> {
    chars: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn parse_node(&mut self, depth: usize) -> Result<Option<XmlNode>, String> {
        if std::env::var("MSM_XML_DEBUG").is_ok() {
            use std::sync::atomic::{AtomicU64, Ordering};
            static CALLS: AtomicU64 = AtomicU64::new(0);
            let calls = CALLS.fetch_add(1, Ordering::Relaxed);
            if calls % 20000 == 0 {
                eprintln!("[xml] calls={} pos={}/{} depth={}", calls, self.pos, self.chars.len(), depth);
            }
        }
        if depth > 128 {
            return Err("XML nesting too deep".into());
        }
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return Ok(None);
        }
        if self.starts_with(b"<?") {
            self.skip_until(b"?>");
            return self.parse_node(depth);
        }
        if self.starts_with(b"<!--") {
            self.skip_until(b"-->");
            return self.parse_node(depth);
        }
        if !self.starts_with(b"<") {
            // Consume the text up to the next tag; returning without advancing
            // made the top-level loop spin forever on a leading BOM.
            let start = self.pos;
            while self.pos < self.chars.len() && self.chars[self.pos] != b'<' {
                self.pos += 1;
            }
            let text = decode_entities(&self.slice(start, self.pos));
            return Ok(Some(XmlNode { name: XmlName { local_name: "#text".into() }, attributes: Vec::new(), children: Vec::new(), text }));
        }
        self.pos += 1;
        let name_start = self.pos;
        while self.pos < self.chars.len() && !self.chars[self.pos].is_ascii_whitespace() && self.chars[self.pos] != b'>' && self.chars[self.pos] != b'/' {
            self.pos += 1;
        }
        let raw_name = self.slice(name_start, self.pos);
        let local_name = raw_name.rsplit(':').next().unwrap_or(raw_name.as_str()).to_string();
        let mut attributes = Vec::new();
        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                return Err("Unexpected end of XML".into());
            }
            if self.starts_with(b"/>") {
                self.pos += 2;
                return Ok(Some(XmlNode { name: XmlName { local_name }, attributes, children: Vec::new(), text: String::new() }));
            }
            if self.starts_with(b">") {
                self.pos += 1;
                break;
            }
            let attr_start = self.pos;
            while self.pos < self.chars.len() && !self.chars[self.pos].is_ascii_whitespace() && self.chars[self.pos] != b'=' {
                self.pos += 1;
            }
            let attr_raw = self.slice(attr_start, self.pos);
            self.skip_ws();
            if !self.starts_with(b"=") {
                if attr_raw.is_empty() { self.pos += 1; continue; }
                attributes.push(XmlAttribute { name: XmlName { local_name: attr_raw.rsplit(':').next().unwrap_or(attr_raw.as_str()).to_string() }, value: String::new() });
                continue;
            }
            self.pos += 1;
            self.skip_ws();
            let quote = self.chars.get(self.pos).copied().unwrap_or(b'"');
            if quote == b'"' || quote == b'\'' {
                self.pos += 1;
                let value_start = self.pos;
                while self.pos < self.chars.len() && self.chars[self.pos] != quote {
                    self.pos += 1;
                }
                let value = self.slice(value_start, self.pos);
                self.pos += 1;
                attributes.push(XmlAttribute { name: XmlName { local_name: attr_raw.rsplit(':').next().unwrap_or(attr_raw.as_str()).to_string() }, value: decode_entities(&value) });
            }
        }
        let mut children = Vec::new();
        let mut text = String::new();
        loop {
            if self.pos >= self.chars.len() {
                return Err("Unexpected end of XML element".into());
            }
            if self.starts_with(b"</") {
                // skip_until already consumes through '>'; the extra +=1 ate the
                // next tag's '<' and desynchronised every close-tag after it.
                self.skip_until(b">");
                return Ok(Some(XmlNode { name: XmlName { local_name }, attributes, children, text }));
            }
            if self.starts_with(b"<!--") {
                self.skip_until(b"-->");
                continue;
            }
            if self.starts_with(b"<![CDATA[") {
                self.pos += 9;
                let start = self.pos;
                while self.pos < self.chars.len() && !self.starts_with(b"]]>") {
                    self.pos += 1;
                }
                text.push_str(&self.slice(start, self.pos));
                self.pos += 3;
                continue;
            }
            if self.starts_with(b"<") {
                if let Some(child) = self.parse_node(depth + 1)? {
                    if child.name.local_name == "#text" {
                        continue;
                    }
                    children.push(child);
                }
                continue;
            }
            let start = self.pos;
            while self.pos < self.chars.len() && self.chars[self.pos] != b'<' {
                self.pos += 1;
            }
            text.push_str(&decode_entities(&self.slice(start, self.pos)));
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && (self.chars[self.pos] as char).is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn starts_with(&self, prefix: &[u8]) -> bool {
        self.chars.len() >= self.pos + prefix.len() && &self.chars[self.pos..self.pos + prefix.len()] == prefix
    }

    fn skip_until(&mut self, marker: &[u8]) {
        while self.pos < self.chars.len() {
            if self.starts_with(marker) {
                // Consume the marker itself; leaving the cursor on it made the
                // top-level loop re-read a degenerate text node forever.
                self.pos += marker.len();
                return;
            }
            self.pos += 1;
        }
    }

    fn slice(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.chars[start.min(end)..end]).into_owned()
    }
}

fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_string();
    }
    value.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

pub fn find_child<'a>(node: &'a XmlNode, local_name: &str) -> Option<&'a XmlNode> {
    node.children.iter().find(|c| eq_ci(&c.name.local_name, local_name))
}

pub fn find_all<'a>(node: &'a XmlNode, local_name: &str) -> Vec<&'a XmlNode> {
    let mut out = Vec::new();
    fn walk<'a>(node: &'a XmlNode, local_name: &str, out: &mut Vec<&'a XmlNode>) {
        for child in &node.children {
            if eq_ci(&child.name.local_name, local_name) {
                out.push(child);
            }
            walk(child, local_name, out);
        }
    }
    walk(node, local_name, &mut out);
    out
}

pub fn child_text(node: &XmlNode, local_name: &str) -> String {
    find_child(node, local_name).map(|c| c.text.trim().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod family_name_tests {
    use super::*;

    #[test]
    fn family_name_keeps_underscore_before_publisher_id() {
        assert_eq!(
            package_family_from_full_name("5319275A.WhatsAppDesktop_2.2644.101.0_x64__cv1g1gvanyjgm"),
            "5319275A.WhatsAppDesktop_cv1g1gvanyjgm"
        );
        assert_eq!(
            packaged_startup_task_state_path("5319275A.WhatsAppDesktop_cv1g1gvanyjgm!2defd21c").unwrap(),
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\SystemAppData\5319275A.WhatsAppDesktop_cv1g1gvanyjgm\2defd21c"
        );
    }
}
