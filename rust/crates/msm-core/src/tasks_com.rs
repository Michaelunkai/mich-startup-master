//! Task Scheduler COM I/O: enumeration walk, single-task readback, state and XML queries.

use crate::model::error_item;
use crate::model::StartupItem;
use crate::tasks::{
    normalize_scheduled_task_executable, normalize_scheduled_task_path, scheduled_task_read_warning,
    try_append_scheduled_task_snapshot, ActionSnapshot, TaskSnapshot, TriggerSnapshot,
    MAXIMUM_SCHEDULED_TASKS, MAXIMUM_TASK_FOLDER_DEPTH, MAXIMUM_TASK_FOLDERS,
};

/// HRESULT codes that mean "the task does not exist" (C# `IsMissingScheduledTask`).
pub fn is_missing_scheduled_task_hresult(code: i32) -> bool {
    let code = code as u32;
    code == 0x80070002 || code == 0x80070003 || code == 0x8004130F
}

/// The whole COM surface runs on one dedicated STA thread so a wedged scheduler
/// cannot block the caller.
fn with_task_com<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    std::thread::Builder::new().name("task-com".into())
        .spawn(move || {
            unsafe {
                let com = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
                if com.is_err() {
                    return Err(format!("COM initialization failed: HRESULT=0x{:08X}", com.0));
                }
                let result = work();
                windows::Win32::System::Com::CoUninitialize();
                result
            }
        })
        .map_err(|e| format!("task COM thread spawn failed: {}", e))?
        .join()
        .map_err(|_| "task COM thread panicked".to_string())?
}

fn connect_task_service() -> Result<windows::Win32::System::TaskScheduler::ITaskService, String> {
    use windows::Win32::System::Com::CLSCTX_INPROC_SERVER;
    use windows::Win32::System::TaskScheduler::{ITaskService, TaskScheduler};
    unsafe {
        let service: ITaskService = windows::Win32::System::Com::CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("Task Scheduler COM service is unavailable: {}", e))?;
        let empty = windows::Win32::System::Variant::VARIANT::default();
        service.Connect(&empty, &empty, &empty, &empty)
            .map_err(|e| format!("Task Scheduler connect failed: {}", e))?;
        Ok(service)
    }
}

pub fn split_task_location_pub(full_task_name: &str) -> Result<(String, String), String> {
    split_task_location(full_task_name)
}

fn split_task_location(full_task_name: &str) -> Result<(String, String), String> {
    let mut normalized = full_task_name.trim().to_string();
    if !normalized.starts_with('\\') {
        normalized = format!("\\{}", normalized.trim_start_matches('\\'));
    }
    normalized = normalized.trim_end_matches('\\').to_string();
    let (folder_path, task_name) = match normalized.rfind('\\') {
        Some(index) => (normalized[..index].to_string(), normalized[index + 1..].to_string()),
        None => ("\\".to_string(), normalized.trim_matches('\\').to_string()),
    };
    if task_name.trim().is_empty() {
        return Err("Task name is required".into());
    }
    Ok((folder_path, task_name))
}

/// Read one task's exact live state through COM (truth readback + state queries).
pub fn werr(context: &str, e: windows::core::Error) -> String {
    format!("{}: HRESULT=0x{:08X}: {}", context, e.code().0, e)
}

/// Read one task's exact live state through COM (truth readback + state queries).
pub fn readback_task(location: &str) -> Result<(String, String, bool, Vec<bool>), String> {
    let location = location.to_string();
    with_task_com(move || unsafe {
        use windows::core::Interface;
        use windows::Win32::System::TaskScheduler::{IAction, IActionCollection, IExecAction, ITaskDefinition, ITrigger, ITriggerCollection};
        let service = connect_task_service()?;
        let (folder_path, task_name) = split_task_location(&location)?;
        let folder = service.GetFolder(&windows::core::BSTR::from(folder_path.as_str()))
            .map_err(|e| format!("GetFolder failed: {}", e))?;
        let task = folder.GetTask(&windows::core::BSTR::from(task_name.as_str()))
            .map_err(|e| format!("GetTask failed: {}", e))?;
        let definition: ITaskDefinition = task.Definition().map_err(|e| format!("Definition read failed: {}", e))?;
        let actions: IActionCollection = definition.Actions().map_err(|e| format!("Actions read failed: {}", e))?;
        let mut count = 0i32;
        actions.Count(&mut count).map_err(|e| format!("Actions count failed: {}", e))?;
        if count != 1 {
            return Err("Multiple or non-executable actions require independent target verification".into());
        }
        let action: IAction = actions.get_Item(1).map_err(|e| format!("Actions item failed: {}", e))?;
        let mut action_kind = windows::Win32::System::TaskScheduler::TASK_ACTION_TYPE(0);
        action.Type(&mut action_kind).map_err(|e| format!("Action type failed: {}", e))?;
        if action_kind.0 != 0 {
            return Err("Multiple or non-executable actions require independent target verification".into());
        }
        let exec: IExecAction = action.cast().map_err(|e| format!("Exec cast failed: {}", e))?;
        let mut execute = windows::core::BSTR::new();
        exec.Path(&mut execute).map_err(|e| format!("Action path failed: {}", e))?;
        let mut arguments = windows::core::BSTR::new();
        exec.Arguments(&mut arguments).map_err(|e| format!("Action arguments failed: {}", e))?;
        let triggers: ITriggerCollection = definition.Triggers().map_err(|e| format!("Triggers read failed: {}", e))?;
        let mut trigger_count = 0i32;
        triggers.Count(&mut trigger_count).map_err(|e| format!("Trigger count failed: {}", e))?;
        let mut trigger_enabled = Vec::new();
        for i in 1..=trigger_count {
            let trigger: ITrigger = triggers.get_Item(i).map_err(|e| format!("Trigger item failed: {}", e))?;
            let mut enabled = windows::Win32::Foundation::VARIANT_BOOL(0);
            trigger.Enabled(&mut enabled).map_err(|e| format!("Trigger enabled failed: {}", e))?;
            trigger_enabled.push(enabled.as_bool());
        }
        let enabled = task.Enabled().map_err(|e| format!("Task enabled failed: {}", e))?.as_bool();
        Ok((execute.to_string(), arguments.to_string(), enabled, trigger_enabled))
    })
}

/// C# `QueryTaskState` — "enabled" | "disabled" | None (missing).
pub fn query_task_state(location: &str) -> Option<String> {
    let location = location.to_string();
    let result = with_task_com(move || unsafe {
        let service = connect_task_service()?;
        let (folder_path, task_name) = split_task_location(&location)?;
        let folder = service.GetFolder(&windows::core::BSTR::from(folder_path.as_str())).map_err(|e| werr("GetFolder", e))?;
        let task = folder.GetTask(&windows::core::BSTR::from(task_name.as_str())).map_err(|e| werr("GetTask", e))?;
        let enabled = task.Enabled().map_err(|e| werr("Enabled", e))?.as_bool();
        Ok(if enabled { "enabled".to_string() } else { "disabled".to_string() })
    });
    result.ok()
}

/// C# `TryReadTaskXml` — exact task XML through COM.
pub fn try_read_task_xml(location: &str) -> Option<String> {
    let location = location.to_string();
    let result = with_task_com(move || unsafe {
        let service = connect_task_service()?;
        let (folder_path, task_name) = split_task_location(&location)?;
        let folder = service.GetFolder(&windows::core::BSTR::from(folder_path.as_str())).map_err(|e| werr("GetFolder", e))?;
        let task = folder.GetTask(&windows::core::BSTR::from(task_name.as_str())).map_err(|e| werr("GetTask", e))?;
        let xml = task.Xml().map_err(|e| werr("Xml", e))?.to_string();
        Ok(xml)
    });
    result.ok()
}

/// C# `EnumerateScheduledTasksCom` — the primary Task Scheduler COM walk.
pub fn enumerate_scheduled_tasks_com(rows: &mut Vec<StartupItem>) {
    let result = with_task_com(move || unsafe {
        let service = connect_task_service()?;
        let root = service.GetFolder(&windows::core::BSTR::from("\\"))
            .map_err(|e| format!("GetFolder failed: {}", e))?;
        let mut rows: Vec<StartupItem> = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        let mut folder_count = 0usize;
        let mut task_count = 0usize;
        enumerate_task_folder_com(&root, "\\", &mut rows, &mut seen, 0, &mut folder_count, &mut task_count);
        Ok(rows)
    });
    match result {
        Ok(mut items) => rows.append(&mut items),
        Err(e) => rows.push(error_item("Scheduled Task", "System", "Task Scheduler COM", &e)),
    }
}

unsafe fn enumerate_task_folder_com(folder: &windows::Win32::System::TaskScheduler::ITaskFolder,
                                     fallback_path: &str, rows: &mut Vec<StartupItem>, seen: &mut Vec<String>,
                                     depth: usize, folder_count: &mut usize, task_count: *mut usize) {
    let task_count = unsafe { &mut *task_count };
    enumerate_task_folder_com_inner(folder, fallback_path, rows, seen, depth, folder_count, task_count)
}

unsafe fn enumerate_task_folder_com_inner(folder: &windows::Win32::System::TaskScheduler::ITaskFolder,
                                           fallback_path: &str, rows: &mut Vec<StartupItem>, seen: &mut Vec<String>,
                                           depth: usize, folder_count: &mut usize, task_count: &mut usize) {
    use windows::Win32::System::TaskScheduler::{ITaskFolder, ITaskFolderCollection};
    let mut folder_path = normalize_scheduled_task_path(fallback_path);
    if let Ok(reported) = folder.Path() {
        let reported = reported.to_string();
        if !reported.trim().is_empty() {
            folder_path = normalize_scheduled_task_path(&reported);
        }
    }
    *folder_count += 1;
    if depth > MAXIMUM_TASK_FOLDER_DEPTH || *folder_count > MAXIMUM_TASK_FOLDERS {
        rows.push(scheduled_task_read_warning(&folder_path, "Folder traversal budget",
            "Task Scheduler folder traversal exceeded its safety budget"));
        return;
    }
    let tasks = match folder.GetTasks(1 /* TASK_ENUM_HIDDEN */) {
        Ok(tasks) => tasks,
        Err(e) => {
            rows.push(scheduled_task_read_warning(&folder_path, "Registered task collection", &format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
            return;
        }
    };
    let count = tasks.Count().unwrap_or(0);
    for i in 1..=count {
        *task_count += 1;
        if *task_count > MAXIMUM_SCHEDULED_TASKS {
            rows.push(scheduled_task_read_warning(&folder_path, "Task traversal budget",
                "Task Scheduler task traversal exceeded its safety budget"));
            break;
        }
        let index = windows::Win32::System::Variant::VARIANT::from(i);
        let task = match tasks.get_Item(&index) {
            Ok(task) => task,
            Err(e) => {
                rows.push(scheduled_task_read_warning(&format!("{}\\#{}", folder_path, i), "Registered task", &format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
                continue;
            }
        };
        match read_scheduled_task_com(&task, &format!("{}\\#{}", folder_path, i), rows) {
            Ok(Some(snapshot)) => { try_append_scheduled_task_snapshot(rows, seen, &snapshot); }
            Ok(None) => {}
            Err((stage, message)) => {
                rows.push(scheduled_task_read_warning(&format!("{}\\#{}", folder_path, i), &stage, &message));
            }
        }
    }
    let folders: ITaskFolderCollection = match folder.GetFolders(0) {
        Ok(folders) => folders,
        Err(e) => {
            rows.push(scheduled_task_read_warning(&folder_path, "Child folder collection", &format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
            return;
        }
    };
    let folder_count_value = folders.Count().unwrap_or(0);
    for i in 1..=folder_count_value {
        let index = windows::Win32::System::Variant::VARIANT::from(i);
        let child: ITaskFolder = match folders.get_Item(&index) {
            Ok(child) => child,
            Err(e) => {
                rows.push(scheduled_task_read_warning(&format!("{}\\#folder{}", folder_path, i), "Child folder", &format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
                continue;
            }
        };
        let mut child_path = format!("{}\\#folder{}", folder_path, i);
        if let Ok(reported) = child.Path() {
            let reported = reported.to_string();
            if !reported.trim().is_empty() {
                child_path = normalize_scheduled_task_path(&reported);
            }
        }
        enumerate_task_folder_com_inner(&child, &child_path, rows, seen, depth + 1, folder_count, task_count);
    }
}

type ComStageError = (String, String);

unsafe fn read_scheduled_task_com(task: &windows::Win32::System::TaskScheduler::IRegisteredTask,
                                   fallback_path: &str, rows: &mut Vec<StartupItem>) -> Result<Option<TaskSnapshot>, ComStageError> {
    use windows::core::Interface;
    use windows::Win32::System::TaskScheduler::{
        IAction, IActionCollection, IComHandlerAction, IExecAction, ILogonTrigger, ITaskDefinition,
        ITrigger, ITriggerCollection,
    };
    let herr = |stage: &str, e: windows::core::Error| (stage.to_string(), format!("HRESULT=0x{:08X}: {}", e.code().0, e));
    let _ = fallback_path;
    let reported_path = task.Path().map_err(|e| herr("Path", e))?.to_string();
    if reported_path.trim().is_empty() {
        return Err(("Path".into(), "Registered task returned an empty path".into()));
    }
    let task_path = normalize_scheduled_task_path(&reported_path);
    let definition: ITaskDefinition = task.Definition().map_err(|e| herr("Definition", e))?;
    let mut snapshot = TaskSnapshot {
        path: task_path.clone(),
        startup_trigger: false,
        task_enabled: true,
        state_label: "Unknown".into(),
        actions_complete: true,
        triggers: Vec::new(),
        actions: Vec::new(),
    };
    let mut trigger_errors: Vec<(String, String)> = Vec::new();
    if let Ok(triggers) = definition.Triggers() {
        let triggers: ITriggerCollection = triggers;
        let mut count = 0i32;
        if triggers.Count(&mut count).is_ok() {
            for i in 1..=count {
                let trigger: ITrigger = match triggers.get_Item(i) {
                    Ok(t) => t,
                    Err(e) => { trigger_errors.push((format!("Trigger[{}]", i), format!("HRESULT=0x{:08X}: {}", e.code().0, e))); continue; }
                };
                let mut type_code = -1i32;
                let mut type_unknown = false;
                let mut trigger_kind = windows::Win32::System::TaskScheduler::TASK_TRIGGER_TYPE2(0);
                match trigger.Type(&mut trigger_kind) {
                    Ok(()) => type_code = trigger_kind.0 as i32,
                    Err(e) => {
                        type_unknown = true;
                        trigger_errors.push((format!("Trigger[{}].Type", i), format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
                    }
                }
                let mut id = String::new();
                let mut id_bstr = windows::core::BSTR::new();
                match trigger.Id(&mut id_bstr) {
                    Ok(()) => id = id_bstr.to_string(),
                    Err(e) => {
                        if is_startup_trigger(type_code, type_unknown) {
                            trigger_errors.push((format!("Trigger[{}].Id", i), format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
                        }
                    }
                }
                let mut enabled = true;
                let mut enabled_unknown = false;
                let mut enabled_value = windows::Win32::Foundation::VARIANT_BOOL(0);
                match trigger.Enabled(&mut enabled_value) {
                    Ok(()) => enabled = enabled_value.as_bool(),
                    Err(e) => {
                        enabled_unknown = true;
                        if is_startup_trigger(type_code, type_unknown) {
                            trigger_errors.push((format!("Trigger[{}].Enabled", i), format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
                        }
                    }
                }
                let mut delay = String::new();
                {
                    use windows::Win32::System::TaskScheduler::{IBootTrigger, ILogonTrigger, ISessionStateChangeTrigger};
                    let delay_result: Option<windows::core::Result<windows::core::BSTR>> = match type_code {
                        8 => trigger.cast::<IBootTrigger>().ok().map(|t| read_trigger_delay(|out| unsafe { t.Delay(out) })),
                        9 => trigger.cast::<ILogonTrigger>().ok().map(|t| read_trigger_delay(|out| unsafe { t.Delay(out) })),
                        11 => trigger.cast::<ISessionStateChangeTrigger>().ok().map(|t| read_trigger_delay(|out| unsafe { t.Delay(out) })),
                        _ => None,
                    };
                    match delay_result {
                        Some(Ok(value)) => delay = value.to_string(),
                        Some(Err(e)) => {
                            if is_startup_trigger(type_code, type_unknown) {
                                trigger_errors.push((format!("Trigger[{}].Delay", i), format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
                            }
                        }
                        None => {}
                    }
                }
                let mut user_id = String::new();
                if type_code == 9 && !type_unknown {
                    if let Ok(logon) = trigger.cast::<ILogonTrigger>() {
                        let mut user_bstr = windows::core::BSTR::new();
                        match logon.UserId(&mut user_bstr) {
                            Ok(()) => user_id = user_bstr.to_string(),
                            Err(e) => trigger_errors.push((format!("Trigger[{}].UserId", i), format!("HRESULT=0x{:08X}: {}", e.code().0, e))),
                        }
                    }
                }
                snapshot.triggers.push(TriggerSnapshot { index: i as usize, type_code, id, enabled, enabled_unknown, type_unknown, delay, user_id, schedule: String::new() });
            }
        }
    }
    // Schedule times (days-of-week + clock time) come from the definition XML:
    // one Xml() read per task feeds every trigger's human schedule text.
    if let Ok(xml) = task.Xml() {
        let xml = xml.to_string();
        if let Ok(doc) = crate::packaged::xml_doc(&xml) {
            if let Some(root) = doc.root.as_ref() {
                if let Some(container) = crate::packaged::find_child(root, "Triggers") {
                    for (i, node) in container.children.iter().enumerate() {
                        let schedule = crate::tasks::trigger_schedule_text(node, &node.name.local_name);
                        if schedule.is_empty() { continue; }
                        if let Some(trigger) = snapshot.triggers.iter_mut().find(|t| t.index == i + 1) {
                            trigger.schedule = schedule;
                        }
                    }
                }
            }
        }
    }
    let startup = snapshot.triggers.iter().any(|t| is_startup_trigger(t.type_code, t.type_unknown));
    for (stage, message) in trigger_errors {
        rows.push(scheduled_task_read_warning(&task_path, &stage, &message));
    }
    snapshot.startup_trigger = startup;
    match task.Enabled() {
        Ok(value) => snapshot.task_enabled = value.as_bool(),
        Err(e) => {
            snapshot.task_enabled = false;
            rows.push(scheduled_task_read_warning(&task_path, "Enabled state", &format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
        }
    }
    match task.State() {
        Ok(state) => snapshot.state_label = format!("{}({})", crate::tasks::task_state_name(state.0 as i32), state.0),
        Err(_) => snapshot.state_label = "Unknown".into(),
    }
    if let Ok(actions) = definition.Actions() {
        let actions: IActionCollection = actions;
        let mut count = 0i32;
        if actions.Count(&mut count).is_ok() {
            for i in 1..=count {
                let action: IAction = match actions.get_Item(i) {
                    Ok(a) => a,
                    Err(e) => {
                        snapshot.actions_complete = false;
                        rows.push(scheduled_task_read_warning(&task_path, &format!("Action[{}]", i), &format!("HRESULT=0x{:08X}: {}", e.code().0, e)));
                        continue;
                    }
                };
                let mut action_kind = windows::Win32::System::TaskScheduler::TASK_ACTION_TYPE(0);
                let action_type = match action.Type(&mut action_kind) {
                    Ok(()) => action_kind.0 as i32,
                    Err(_) => -1,
                };
                let mut action_snapshot = ActionSnapshot {
                    index: i as usize, type_code: action_type, kind: String::new(),
                    path: String::new(), arguments: String::new(), working_directory: String::new(),
                    class_id: String::new(), data: String::new(),
                };
                if action_type == 0 {
                    if let Ok(exec) = action.cast::<IExecAction>() {
                        action_snapshot.kind = "Exec".into();
                        let mut value = windows::core::BSTR::new();
                        if exec.Path(&mut value).is_ok() {
                            action_snapshot.path = normalize_scheduled_task_executable(&value.to_string());
                        }
                        if exec.Arguments(&mut value).is_ok() {
                            action_snapshot.arguments = value.to_string();
                        }
                        if exec.WorkingDirectory(&mut value).is_ok() {
                            action_snapshot.working_directory = value.to_string();
                        }
                    }
                } else if action_type == 5 {
                    if let Ok(handler) = action.cast::<IComHandlerAction>() {
                        action_snapshot.kind = "ComHandler".into();
                        let mut value = windows::core::BSTR::new();
                        if handler.ClassId(&mut value).is_ok() {
                            action_snapshot.class_id = value.to_string();
                        }
                        if handler.Data(&mut value).is_ok() {
                            action_snapshot.data = value.to_string();
                        }
                    }
                } else {
                    action_snapshot.kind = format!("Type{}", action_type);
                }
                snapshot.actions.push(action_snapshot);
            }
        }
    }
    Ok(Some(snapshot))
}

fn is_startup_trigger(type_code: i32, type_unknown: bool) -> bool {
    type_unknown || type_code == 8 || type_code == 9 || type_code == 11
}

fn read_trigger_delay(read: impl FnOnce(*mut windows::core::BSTR) -> windows::core::Result<()>) -> windows::core::Result<windows::core::BSTR> {
    let mut value = windows::core::BSTR::new();
    unsafe { read(&mut value as *mut _)? };
    Ok(value)
}

/// Batch truth-readback for a whole scan: ONE COM connection for every task
/// (per-task connections make a full-machine verification take minutes).
pub fn readback_tasks_batch(locations: &[String]) -> std::collections::HashMap<String, (String, String, bool, Vec<bool>)> {
    let owned: Vec<String> = locations.to_vec();
    let result = with_task_com(move || unsafe {
        use windows::core::Interface;
        use windows::Win32::System::TaskScheduler::{IAction, IActionCollection, IExecAction, ITaskDefinition, ITrigger, ITriggerCollection};
        let service = connect_task_service()?;
        let root = service.GetFolder(&windows::core::BSTR::from("\\"))
            .map_err(|e| werr("GetFolder", e))?;
        let mut map: std::collections::HashMap<String, (String, String, bool, Vec<bool>)> = std::collections::HashMap::new();
        for location in &owned {
            let Ok((folder_path, task_name)) = split_task_location(location) else { continue };
            let folder = if folder_path == "\\" {
                root.clone()
            } else {
                match service.GetFolder(&windows::core::BSTR::from(folder_path.as_str())) {
                    Ok(folder) => folder,
                    Err(_) => continue,
                }
            };
            let Ok(task) = folder.GetTask(&windows::core::BSTR::from(task_name.as_str())) else { continue };
            let Ok(definition) = task.Definition() else { continue };
            let Ok(actions) = definition.Actions() else { continue };
            let mut count = 0i32;
            if actions.Count(&mut count).is_err() { continue }
            if count != 1 { continue }
            let Ok(action) = actions.get_Item(1) else { continue };
            let mut action_kind = windows::Win32::System::TaskScheduler::TASK_ACTION_TYPE(0);
            if action.Type(&mut action_kind).is_err() || action_kind.0 != 0 { continue }
            let Ok(exec) = action.cast::<IExecAction>() else { continue };
            let mut execute = windows::core::BSTR::new();
            if exec.Path(&mut execute).is_err() { continue }
            let mut arguments = windows::core::BSTR::new();
            if exec.Arguments(&mut arguments).is_err() { continue }
            let mut trigger_enabled: Vec<bool> = Vec::new();
            if let Ok(triggers) = definition.Triggers() {
                let triggers: ITriggerCollection = triggers;
                let mut trigger_count = 0i32;
                if triggers.Count(&mut trigger_count).is_ok() {
                    for i in 1..=trigger_count {
                        if let Ok(trigger) = triggers.get_Item(i) {
                            let mut enabled = windows::Win32::Foundation::VARIANT_BOOL(0);
                            if trigger.Enabled(&mut enabled).is_ok() {
                                trigger_enabled.push(enabled.as_bool());
                            }
                        }
                    }
                }
            }
            let Ok(enabled) = task.Enabled() else { continue };
            map.insert(location.clone(), (execute.to_string(), arguments.to_string(), enabled.as_bool(), trigger_enabled));
        }
        Ok(map)
    });
    result.unwrap_or_default()
}
