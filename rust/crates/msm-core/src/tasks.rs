//! Scheduled tasks: COM inventory walk + independent XML file audit.

use crate::ids;
use crate::model::{error_item, StartupItem};
use crate::packaged::{child_text, find_all, find_child, xml_doc, XmlNode};
use crate::util::eq_ci;

pub const MAXIMUM_TASK_FOLDERS: usize = 2048;
pub const MAXIMUM_SCHEDULED_TASKS: usize = 16384;
pub const MAXIMUM_TASK_FOLDER_DEPTH: usize = 64;

#[derive(Debug, Clone)]
pub struct TriggerSnapshot {
    pub index: usize,
    pub type_code: i32,
    pub id: String,
    pub enabled: bool,
    pub delay: String,
    pub user_id: String,
    pub type_unknown: bool,
    pub enabled_unknown: bool,
    /// Human schedule text ("Weekly · Mon, Fri · 09:00", "At log on", ...).
    pub schedule: String,
}

#[derive(Debug, Clone)]
pub struct ActionSnapshot {
    pub index: usize,
    pub type_code: i32,
    pub kind: String,
    pub path: String,
    pub arguments: String,
    pub working_directory: String,
    pub class_id: String,
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct TaskSnapshot {
    pub path: String,
    pub startup_trigger: bool,
    pub task_enabled: bool,
    pub state_label: String,
    pub actions_complete: bool,
    pub triggers: Vec<TriggerSnapshot>,
    pub actions: Vec<ActionSnapshot>,
}

pub fn is_startup_task_trigger_type(value: i32) -> bool {
    value == 8 || value == 9 || value == 11
}

pub fn task_trigger_name(value: i32) -> String {
    match value {
        0 => "Event".into(), 1 => "Time".into(), 2 => "Daily".into(), 3 => "Weekly".into(),
        4 => "Monthly".into(), 5 => "MonthlyDOW".into(), 6 => "Idle".into(), 7 => "Registration".into(),
        8 => "Boot".into(), 9 => "Logon".into(), 11 => "SessionState".into(),
        other => format!("Type{}", other),
    }
}

pub fn task_state_name(state: i32) -> String {
    match state {
        0 => "Unknown".into(), 1 => "Disabled".into(), 2 => "Queued".into(), 3 => "Ready".into(),
        4 => "Running".into(), other => format!("State{}", other),
    }
}

/// Strip outer quote pairs from a Task Scheduler Exec command.
pub fn normalize_scheduled_task_executable(value: &str) -> String {
    let mut normalized = value.trim().to_string();
    while normalized.len() >= 2 && normalized.starts_with('"') && normalized.ends_with('"') {
        let inner = normalized[1..normalized.len() - 1].trim().to_string();
        if inner.len() < 2 {
            break;
        }
        normalized = inner;
    }
    normalized
}

pub fn normalize_scheduled_task_path(value: &str) -> String {
    let text = value.trim().replace('/', "\\");
    let parts: Vec<&str> = text.split('\\').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() { "\\".to_string() } else { format!("\\{}", parts.join("\\")) }
}

pub fn task_path_from_definition_file(root: &str, file: &str) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file).replace('/', "\\");
    normalize_scheduled_task_path(&relative)
}

fn task_detail_value(value: &str) -> String {
    value.replace('\r', "\\r").replace('\n', "\\n").replace(';', "\\;").replace('|', "\\|")
}

pub fn task_action_command(action: &ActionSnapshot) -> String {
    if action.type_code == 0 {
        let exe = crate::util::win_arg(&normalize_scheduled_task_executable(&action.path));
        return format!("{}{}", exe, if action.arguments.trim().is_empty() { String::new() } else { format!(" {}", action.arguments) })
            .trim().to_string();
    }
    if action.type_code == 5 {
        return format!("COM handler {}", action.class_id);
    }
    if action.kind.is_empty() { format!("Task action type {}", action.type_code) } else { format!("Task action {}", action.kind) }
}

fn task_action_details(action: &ActionSnapshot) -> String {
    if action.type_code == 0 {
        format!("Exec(index={};path={};arguments={};workingDirectory={})",
            action.index, task_detail_value(&action.path), task_detail_value(&action.arguments), task_detail_value(&action.working_directory))
    } else if action.type_code == 5 {
        format!("ComHandler(index={};classId={};data={})",
            action.index, task_detail_value(&action.class_id), task_detail_value(&action.data))
    } else {
        format!("Action(index={};kind={};type={})", action.index, task_detail_value(&action.kind), action.type_code)
    }
}

/// C# `ParseScheduledTaskDocument` — XML definition → snapshot.
pub fn parse_scheduled_task_document(task_path: &str, root: &XmlNode) -> TaskSnapshot {
    let settings_enabled = find_child(root, "Settings").and_then(|s| find_child(s, "Enabled"))
        .map(|e| e.text.trim().parse::<bool>().unwrap_or(true)).unwrap_or(true);
    let mut triggers = Vec::new();
    if let Some(container) = find_child(root, "Triggers") {
        for (i, node) in container.children.iter().enumerate() {
            triggers.push(TriggerSnapshot {
                index: i + 1,
                type_code: task_trigger_type_from_xml_name(&node.name.local_name),
                id: node.attributes.iter().find(|a| eq_ci(&a.name.local_name, "id")).map(|a| a.value.clone()).unwrap_or_default(),
                enabled: find_child(node, "Enabled").map(|e| e.text.trim().parse::<bool>().unwrap_or(true)).unwrap_or(true),
                delay: child_text(node, "Delay"),
                user_id: child_text(node, "UserId"),
                type_unknown: false,
                enabled_unknown: false,
                schedule: trigger_schedule_text(node, &node.name.local_name),
            });
        }
    }
    let mut actions = Vec::new();
    if let Some(container) = find_child(root, "Actions") {
        for (i, node) in container.children.iter().enumerate() {
            if eq_ci(&node.name.local_name, "Exec") {
                actions.push(ActionSnapshot {
                    index: i + 1, type_code: 0, kind: "Exec".into(),
                    path: normalize_scheduled_task_executable(&child_text(node, "Command")),
                    arguments: child_text(node, "Arguments"),
                    working_directory: child_text(node, "WorkingDirectory"),
                    class_id: String::new(), data: String::new(),
                });
            } else if eq_ci(&node.name.local_name, "ComHandler") {
                actions.push(ActionSnapshot {
                    index: i + 1, type_code: 5, kind: "ComHandler".into(),
                    path: String::new(), arguments: String::new(), working_directory: String::new(),
                    class_id: child_text(node, "ClassId"), data: child_text(node, "Data"),
                });
            } else {
                actions.push(ActionSnapshot {
                    index: i + 1, type_code: -1, kind: node.name.local_name.clone(),
                    path: String::new(), arguments: String::new(), working_directory: String::new(),
                    class_id: String::new(), data: String::new(),
                });
            }
        }
    }
    TaskSnapshot {
        path: normalize_scheduled_task_path(task_path),
        startup_trigger: false,
        task_enabled: settings_enabled,
        state_label: "Definition".into(),
        actions_complete: true,
        triggers,
        actions,
    }
}

pub fn parse_scheduled_task_xml(task_path: &str, xml: &str) -> Result<TaskSnapshot, String> {
    let doc = xml_doc(xml)?;
    let root = doc.root.ok_or("Task definition root is missing")?;
    Ok(parse_scheduled_task_document(task_path, &root))
}

fn task_trigger_type_from_xml_name(name: &str) -> i32 {
    if eq_ci(name, "BootTrigger") { 8 }
    else if eq_ci(name, "LogonTrigger") { 9 }
    else if eq_ci(name, "EventTrigger") { 0 }
    else if eq_ci(name, "TimeTrigger") { 1 }
    else if eq_ci(name, "IdleTrigger") { 6 }
    else if eq_ci(name, "RegistrationTrigger") { 7 }
    else if eq_ci(name, "SessionStateChangeTrigger") { 11 }
    else { -1 }
}

/// "2026-01-03T21:45:00(+offset)" → ("21:45", "03 Jan 21:45").
fn boundary_parts(boundary: &str) -> (String, String) {
    let Some((date, time_raw)) = boundary.split_once('T') else { return (String::new(), String::new()) };
    let time = time_raw.split(['+', '-']).next().unwrap_or("").trim_end_matches('Z');
    let hhmm = time.get(..5).unwrap_or(time).to_string();
    if hhmm.len() < 4 { return (String::new(), String::new()); }
    let month = match date.get(5..7).unwrap_or("") {
        "01" => "Jan", "02" => "Feb", "03" => "Mar", "04" => "Apr", "05" => "May", "06" => "Jun",
        "07" => "Jul", "08" => "Aug", "09" => "Sep", "10" => "Oct", "11" => "Nov", "12" => "Dec",
        _ => "",
    };
    let day = date.get(8..10).unwrap_or("").trim_start_matches('0');
    let stamp = if day.is_empty() || month.is_empty() { hhmm.clone() } else { format!("{} {} {}", day, month, hhmm) };
    (hhmm, stamp)
}

/// ISO 8601 duration "PT30S" / "PT1H15M" / "P1D" → "30s" / "1h 15m" / "1d".
fn human_delay(raw: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut number = String::new();
    for c in raw.chars() {
        if c.is_ascii_digit() || c == '.' { number.push(c); continue; }
        let unit = match c {
            'H' | 'h' => "h", 'M' | 'm' => "m", 'S' | 's' => "s", 'D' | 'd' => "d",
            _ => { number.clear(); continue },
        };
        if !number.is_empty() { out.push(format!("{}{}", number, unit)); }
        number.clear();
    }
    out.join(" ")
}

fn weekday_names(days_node: &XmlNode) -> Vec<&'static str> {
    const ORDER: [(&str, &str); 7] = [
        ("Monday", "Mon"), ("Tuesday", "Tue"), ("Wednesday", "Wed"), ("Thursday", "Thu"),
        ("Friday", "Fri"), ("Saturday", "Sat"), ("Sunday", "Sun"),
    ];
    ORDER.iter()
        .filter(|(xml, _)| days_node.children.iter().any(|c| eq_ci(&c.name.local_name, xml)))
        .map(|(_, short)| *short)
        .collect()
}

/// Human schedule text for one XML trigger element — the "time every week"
/// contract: weekly calendars show days + clock time, e.g.
/// "Weekly · Mon, Fri · 09:00".
pub fn trigger_schedule_text(node: &XmlNode, type_name: &str) -> String {
    let boundary = child_text(node, "StartBoundary");
    let (time, stamp) = boundary_parts(&boundary);
    let delay = human_delay(&child_text(node, "Delay"));
    let with_delay = |base: String| if delay.is_empty() { base } else { format!("{} \u{00B7} +{}", base, delay) };
    let time_part = |text: String| if time.is_empty() { text } else { format!("{} \u{00B7} {}", text, time) };
    if eq_ci(type_name, "LogonTrigger") { return with_delay("At log on".into()); }
    if eq_ci(type_name, "BootTrigger") { return with_delay("At boot".into()); }
    if eq_ci(type_name, "IdleTrigger") { return "When idle".into(); }
    if eq_ci(type_name, "EventTrigger") { return "On event".into(); }
    if eq_ci(type_name, "RegistrationTrigger") { return "On registration".into(); }
    if eq_ci(type_name, "SessionStateChangeTrigger") { return "On session change".into(); }
    if eq_ci(type_name, "TimeTrigger") {
        return if stamp.is_empty() { "Once".into() } else { format!("Once \u{00B7} {}", stamp) };
    }
    // Calendar triggers.
    if let Some(week) = find_child(node, "ScheduleByWeek") {
        let days = find_child(week, "DaysOfWeek").map(weekday_names).unwrap_or_default();
        let interval = child_text(week, "WeeksInterval").trim().parse::<u32>().unwrap_or(1);
        let prefix = if interval <= 1 { "Weekly".to_string() } else { format!("Every {} weeks", interval) };
        let days_text = if days.is_empty() { String::new() } else { format!(" \u{00B7} {}", days.join(", ")) };
        return time_part(format!("{}{}", prefix, days_text));
    }
    if let Some(day) = find_child(node, "ScheduleByDay") {
        let interval = child_text(day, "DaysInterval").trim().parse::<u32>().unwrap_or(1);
        let prefix = if interval <= 1 { "Daily".to_string() } else { format!("Every {} days", interval) };
        return time_part(prefix);
    }
    if let Some(month) = find_child(node, "ScheduleByMonth") {
        let days: Vec<String> = find_all(month, "DaysOfMonth").iter()
            .flat_map(|holder| find_all(holder, "Day").iter().map(|d| d.text.trim().to_string()).collect::<Vec<_>>())
            .filter(|d| !d.is_empty()).collect();
        let days_text = if days.is_empty() { String::new() } else { format!(" \u{00B7} day {}", days.join(", ")) };
        return time_part(format!("Monthly{}", days_text));
    }
    if find_child(node, "ScheduleByMonthDOW").is_some() {
        return time_part("Monthly \u{00B7} by weekday".into());
    }
    String::new()
}

/// Joined schedule summary for a task's live (enabled) triggers, falling back
/// to all triggers when every one is disabled.
pub fn task_schedule_summary(triggers: &[TriggerSnapshot]) -> String {
    let live: Vec<&str> = triggers.iter().filter(|t| t.enabled || t.enabled_unknown)
        .map(|t| t.schedule.as_str()).filter(|s| !s.trim().is_empty()).collect();
    let source: Vec<&str> = if live.is_empty() {
        triggers.iter().map(|t| t.schedule.as_str()).filter(|s| !s.trim().is_empty()).collect()
    } else { live };
    let mut deduped: Vec<&str> = Vec::new();
    for text in source {
        if !deduped.iter().any(|d| eq_ci(d, text)) { deduped.push(text); }
    }
    if deduped.is_empty() { String::new() } else { deduped.join(" + ") }
}

/// C# `TryAppendScheduledTaskSnapshot` — emit one row per task with exact status text.
pub fn try_append_scheduled_task_snapshot(rows: &mut Vec<StartupItem>, seen: &mut Vec<String>, snapshot: &TaskSnapshot) -> bool {
    let task_path = normalize_scheduled_task_path(&snapshot.path);
    let startup_triggers: Vec<&TriggerSnapshot> = snapshot.triggers.iter()
        .filter(|t| is_startup_task_trigger_type(t.type_code) || t.type_unknown).collect();
    if task_path.is_empty() || seen.iter().any(|s| eq_ci(s, &task_path)) {
        return false;
    }
    seen.push(task_path.clone());
    let startup_route = !startup_triggers.is_empty();
    let enabled_startup_trigger = startup_triggers.iter().any(|t| t.enabled_unknown || t.enabled);
    let live_triggers: Vec<&TriggerSnapshot> = snapshot.triggers.iter().filter(|t| t.enabled_unknown || t.enabled).collect();
    let enabled = snapshot.task_enabled;
    let live_kinds: Vec<String> = live_triggers.iter().map(|t| task_trigger_name(t.type_code)).collect();
    let live_kinds = {
        let mut deduped: Vec<String> = Vec::new();
        for kind in live_kinds {
            if !deduped.iter().any(|d| eq_ci(d, &kind)) { deduped.push(kind); }
        }
        deduped.join("+")
    };
    let mut kinds: Vec<String> = snapshot.triggers.iter().map(|t| task_trigger_name(t.type_code)).collect();
    kinds.dedup_by(|a, b| eq_ci(a, b));
    let kinds = if kinds.is_empty() { "No-trigger (on-demand)".to_string() } else { kinds.join("+") };
    let trigger_details: Vec<String> = snapshot.triggers.iter().map(|trigger| {
        let mut text = format!("{}(index={};enabled={}", task_trigger_name(trigger.type_code), trigger.index,
            if trigger.enabled_unknown { "unknown".to_string() } else { trigger.enabled.to_string() });
        if !trigger.id.is_empty() {
            text.push_str(&format!(";id={}", task_detail_value(&trigger.id)));
        }
        text.push_str(&format!(";delay={}", task_detail_value(&trigger.delay)));
        if !trigger.user_id.is_empty() {
            text.push_str(&format!(";userId={}", task_detail_value(&trigger.user_id)));
        }
        text.push(')');
        text
    }).collect();
    let command: Vec<String> = snapshot.actions.iter().map(task_action_command).filter(|c| !c.trim().is_empty()).collect();
    let action_details: Vec<String> = snapshot.actions.iter().map(task_action_details).collect();
    let mode_editable = snapshot.actions_complete && snapshot.actions.len() == 1 && snapshot.actions.first().map(|a| a.type_code == 0 && !a.path.trim().is_empty()).unwrap_or(false);
    let mut status = format!("{}{} {}",
        if enabled { "Enabled" } else { "Disabled" },
        kinds,
        if startup_route { " startup task" }
        else if snapshot.triggers.is_empty() { " scheduled task (no trigger; starts only when another program or service runs it)" }
        else { " scheduled task (non-boot trigger)" });
    if snapshot.task_enabled && !snapshot.triggers.is_empty() && live_triggers.is_empty() {
        status.push_str(" (all triggers disabled; runs only when started on demand)");
    }
    if snapshot.task_enabled && startup_route && !enabled_startup_trigger && !live_triggers.is_empty() {
        status.push_str(&format!(" (boot/logon trigger disabled; still runs on {})", live_kinds));
    }
    status.push_str(&format!("; state={}; taskEnabled={}; modeEditable={}; triggers={}; actions={}",
        if snapshot.state_label.is_empty() { "Unknown" } else { &snapshot.state_label },
        snapshot.task_enabled, mode_editable, trigger_details.join(" || "), action_details.join(" || ")));
    let microsoft_library_task = task_path.to_lowercase().starts_with("\\microsoft\\");
    rows.push(StartupItem {
        id: ids::task_id(&task_path),
        name: task_path.trim_start_matches('\\').to_string(),
        app_name: crate::humans::friendly_scheduled_task_name(&task_path),
        source: if startup_route { "Scheduled Task" } else { "Scheduled Task (Any Trigger)" }.into(),
        scope: "User/System".into(),
        command: command.join(" || "),
        location: task_path.clone(),
        enabled,
        can_disable: true,
        is_managed: is_managed_scheduled_task_path(&task_path),
        external_authority: if !startup_route && microsoft_library_task { "Windows task library".into() } else { String::new() },
        status,
        schedule: task_schedule_summary(&snapshot.triggers),
        ..StartupItem::new()
    });
    true
}

pub fn is_managed_scheduled_task_path(task_path: &str) -> bool {
    let normalized = normalize_scheduled_task_path(task_path);
    let root = crate::scan::MANAGED_TASK_ROOT;
    normalized.to_lowercase().starts_with(&root.to_lowercase())
        || normalized.trim_end_matches('\\').eq_ignore_ascii_case(root.trim_end_matches('\\'))
}

pub fn scheduled_task_read_warning(task_path: &str, stage: &str, message: &str) -> StartupItem {
    error_item("Scheduled Task", "System", &format!("{} :: {}", normalize_scheduled_task_path(task_path), stage), message)
}

/// C# `AuditScheduledTaskFiles` — independent XML walk of C:\\Windows\\System32\\Tasks.
pub fn audit_scheduled_task_files(rows: &mut Vec<StartupItem>) {
    let root = format!("{}\\Tasks", crate::paths::system_dir());
    if !std::path::Path::new(&root).is_dir() {
        rows.push(error_item("Independent Scheduled Task", "System", &root, "Task definition catalog was not found"));
        return;
    }
    let mut seen = Vec::new();
    let mut folder_count = 0usize;
    let mut file_count = 0usize;
    audit_task_file_folder(rows, &root, &root, "\\", &mut seen, 0, &mut folder_count, &mut file_count);
}

fn audit_task_file_folder(rows: &mut Vec<StartupItem>, root: &str, folder: &str, relative_folder: &str,
                           seen: &mut Vec<String>, depth: usize, folder_count: &mut usize, file_count: &mut usize) {
    *folder_count += 1;
    if depth > MAXIMUM_TASK_FOLDER_DEPTH || *folder_count > MAXIMUM_TASK_FOLDERS {
        rows.push(error_item("Independent Scheduled Task", "System", &normalize_scheduled_task_path(relative_folder),
            "Task definition folder traversal exceeded its safety budget"));
        return;
    }
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries.filter_map(|e| e.ok()).collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    for entry in &entries {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        *file_count += 1;
        let file = entry.path().to_string_lossy().into_owned();
        let task_path = task_path_from_definition_file(root, &file);
        if *file_count > MAXIMUM_SCHEDULED_TASKS {
            rows.push(error_item("Independent Scheduled Task", "System", &task_path,
                "Task definition file traversal exceeded its safety budget"));
            return;
        }
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        if text.len() > 16 * 1024 * 1024 {
            continue;
        }
        match parse_scheduled_task_xml(&task_path, &text) {
            Ok(mut snapshot) => {
                snapshot.path = task_path.clone();
                snapshot.startup_trigger = snapshot.triggers.iter().any(|t| is_startup_task_trigger_type(t.type_code));
                try_append_scheduled_task_snapshot(rows, seen, &snapshot);
            }
            Err(_) => { /* unreadable candidate: skip silently, coverage proven by COM walk */ }
        }
    }
    let mut children: Vec<_> = entries.iter().filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false)).collect();
    children.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());
    for child in children {
        let path = child.path().to_string_lossy().into_owned();
        let relative = normalize_scheduled_task_path(&path.strip_prefix(root).unwrap_or(&path).replace('/', "\\"));
        let Ok(metadata) = std::fs::symlink_metadata(&path) else { continue };
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            continue; // reparse point
        }
        audit_task_file_folder(rows, root, &path, &relative, seen, depth + 1, folder_count, file_count);
    }
}


#[cfg(test)]
mod schedule_tests {
    use super::*;

    #[test]
    fn weekly_trigger_shows_days_and_time() {
        let xml = "<Task><Triggers><CalendarTrigger><StartBoundary>2026-01-01T09:30:00</StartBoundary>\
<ScheduleByWeek><WeeksInterval>1</WeeksInterval><DaysOfWeek><Monday/><Friday/></DaysOfWeek></ScheduleByWeek>\
</CalendarTrigger></Triggers></Task>";
        let doc = xml_doc(xml).unwrap();
        let root = doc.root.unwrap();
        let snapshot = parse_scheduled_task_document(r"\Test\Weekly", &root);
        assert_eq!(task_schedule_summary(&snapshot.triggers), "Weekly \u{00B7} Mon, Fri \u{00B7} 09:30");
    }

    #[test]
    fn every_two_weeks_daily_and_logon() {
        let biweekly = "<Task><Triggers><CalendarTrigger><StartBoundary>2026-01-01T21:00:00</StartBoundary>\
<ScheduleByWeek><WeeksInterval>2</WeeksInterval><DaysOfWeek><Wednesday/></DaysOfWeek></ScheduleByWeek>\
</CalendarTrigger></Triggers></Task>";
        let doc = xml_doc(biweekly).unwrap();
        let snapshot = parse_scheduled_task_document(r"\Test\Bi", doc.root.as_ref().unwrap());
        assert_eq!(task_schedule_summary(&snapshot.triggers), "Every 2 weeks \u{00B7} Wed \u{00B7} 21:00");

        let logon = "<Task><Triggers><LogonTrigger><Delay>PT30S</Delay></LogonTrigger></Triggers></Task>";
        let doc = xml_doc(logon).unwrap();
        let snapshot = parse_scheduled_task_document(r"\Test\Logon", doc.root.as_ref().unwrap());
        assert_eq!(task_schedule_summary(&snapshot.triggers), "At log on \u{00B7} +30s");

        let daily = "<Task><Triggers><CalendarTrigger><StartBoundary>2026-01-01T06:15:00</StartBoundary>\
<ScheduleByDay><DaysInterval>1</DaysInterval></ScheduleByDay></CalendarTrigger></Triggers></Task>";
        let doc = xml_doc(daily).unwrap();
        let snapshot = parse_scheduled_task_document(r"\Test\Daily", doc.root.as_ref().unwrap());
        assert_eq!(task_schedule_summary(&snapshot.triggers), "Daily \u{00B7} 06:15");
    }

    #[test]
    fn item_carries_schedule_from_snapshot() {
        let xml = r"<Task><Triggers><CalendarTrigger><StartBoundary>2026-01-01T08:00:00</StartBoundary><ScheduleByWeek><DaysOfWeek><Tuesday/><Thursday/></DaysOfWeek></ScheduleByWeek></CalendarTrigger></Triggers><Actions><Exec><Command>C:\Bin\app.exe</Command></Exec></Actions></Task>";
        let doc = xml_doc(xml).unwrap();
        let snapshot = parse_scheduled_task_document(r"\MichStartupMaster\Test", doc.root.as_ref().unwrap());
        let mut rows = Vec::new();
        let mut seen = Vec::new();
        assert!(try_append_scheduled_task_snapshot(&mut rows, &mut seen, &snapshot));
        assert_eq!(rows[0].schedule, "Weekly \u{00B7} Tue, Thu \u{00B7} 08:00");
    }
}
