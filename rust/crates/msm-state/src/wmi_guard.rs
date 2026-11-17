//! WMI binding deletion/recreation guard helpers (root\subscription).

/// Delete a `__FilterToConsumerBinding` by its object path.
pub fn delete_binding(location: &str) -> Result<bool, String> {
    let location = location.to_string();
    let joined = std::thread::Builder::new()
        .name("wmi-delete-binding".into())
        .spawn(move || -> Result<bool, String> {
            let com = wmi_shim_init()?;
            // wmi crate cannot delete by path directly; use raw query for the instance and drop it.
            let _ = com;
            // PowerShell performs the exact WMI deletion with the archived metadata.
            let script = format!(
                "$ErrorActionPreference='Stop';$b=[wmi]('{}');$b.Delete();",
                location.replace('\'', "''")
            );
            msm_core::managed::run_checked(&msm_core::managed::power_shell_exe(),
                &format!("-NoProfile -ExecutionPolicy Bypass -EncodedCommand {}", encode_command(&script)), 20_000)?;
            Ok(true)
        })
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| "WMI delete worker panicked".to_string())?;
    joined.map(|deleted| deleted)
}

/// Does a `__FilterToConsumerBinding` exist at this object path right now?
pub fn binding_exists(location: &str) -> Result<bool, String> {
    let location = location.to_string();
    let joined = std::thread::Builder::new()
        .name("wmi-binding-exists".into())
        .spawn(move || -> Result<bool, String> {
            let com = wmi_shim_init()?;
            let wmi = wmi::WMIConnection::with_namespace_path(r"ROOT\SUBSCRIPTION", com)
                .map_err(|e| format!("WMI connection failed: {}", e))?;
            #[derive(serde::Deserialize, Default)]
            #[serde(rename_all = "PascalCase")]
            struct BindingRow { #[serde(default)] filter: String, #[serde(default)] consumer: String }
            let rows: Vec<BindingRow> = wmi.raw_query("SELECT Filter, Consumer FROM __FilterToConsumerBinding")
                .map_err(|e| format!("binding query failed: {}", e))?;
            Ok(rows.iter().any(|r| {
                // The stored object path embeds the raw Filter/Consumer strings in quotes.
                let rebuilt = format!("__FilterToConsumerBinding.Filter=\"{}{}\",Consumer=\"{}{}\"",
                    r.filter, quote(), r.consumer, quote());
                rebuilt == location
            }))
        })
        .map_err(|e| e.to_string())?
        .join()
        .map_err(|_| "WMI exists worker panicked".to_string())?;
    joined
}

/// Recreate a binding from archived Filter/Consumer references.
pub fn create_binding(filter: &str, consumer: &str) -> Result<(), String> {
    let script = format!(
        "$ErrorActionPreference='Stop';$class=[wmiclass]'\\\\.\\root\\subscription:__FilterToConsumerBinding';$b=$class.CreateInstance();$b.Filter='{filter}';$b.Consumer='{consumer}';$b.Put() | Out-Null;",
        filter = filter.replace('\'', "''"),
        consumer = consumer.replace('\'', "''")
    );
    msm_core::managed::run_checked(&msm_core::managed::power_shell_exe(),
        &format!("-NoProfile -ExecutionPolicy Bypass -EncodedCommand {}", encode_command(&script)), 20_000)
}

fn wmi_shim_init() -> Result<wmi::COMLibrary, String> {
    wmi::COMLibrary::new().map_err(|e| format!("WMI COM initialization failed: {}", e))
}

fn encode_command(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(&utf16)
}

/// A plain double-quote character (keeps the format string above readable).
fn quote() -> char { '"' }

