//! Loaded-user hives and service-account profiles.

use crate::reg::{open_subkey, RegRoot, RegView};
use crate::util::current_user_sid;
use once_cell::sync::Lazy;
use regex::Regex;

pub const SERVICE_ACCOUNT_PROFILES: [(&str, &str); 3] = [
    ("S-1-5-18", "SYSTEM"),
    ("S-1-5-19", "LOCAL SERVICE"),
    ("S-1-5-20", "NETWORK SERVICE"),
];

pub fn is_service_account_sid(sid: &str) -> bool {
    SERVICE_ACCOUNT_PROFILES.iter().any(|(s, _)| s.eq_ignore_ascii_case(sid))
}

static USER_SID_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^S-1-5-21-(?:\d+-){2,}\d+$").unwrap());

/// (sid, scope "User:<name>") pairs for loaded profiles other than the current user,
/// plus readable service-account profiles.
pub fn loaded_user_hives() -> Vec<(String, String)> {
    let mut result = Vec::new();
    let current_sid = current_user_sid();
    let users = match open_subkey(RegRoot::Users, "", RegView::Default) {
        Some(key) => key.subkey_names(),
        None => return result,
    };
    for sid in users {
        if !USER_SID_RE.is_match(&sid) {
            continue;
        }
        if !current_sid.is_empty() && sid.eq_ignore_ascii_case(&current_sid) {
            continue;
        }
        let user = match open_subkey(RegRoot::LocalMachine, &format!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\ProfileList\\{}", sid), RegView::Default) {
            Some(profile) => {
                let raw = profile.value("ProfileImagePath").map(|(_, v)| v.display()).unwrap_or_default();
                let expanded = crate::util::expand_env(&raw);
                let trimmed = expanded.trim_end_matches('\\');
                trimmed.rsplit('\\').next().unwrap_or("").to_string()
            }
            None => String::new(),
        };
        let user = if user.trim().is_empty() {
            if sid.len() > 12 { sid[sid.len() - 12..].to_string() } else { sid.clone() }
        } else {
            user
        };
        result.push((sid, format!("User:{}", user)));
    }
    for (sid, name) in SERVICE_ACCOUNT_PROFILES {
        if !current_sid.is_empty() && sid.eq_ignore_ascii_case(&current_sid) {
            continue;
        }
        if service_profile_state(sid) == "readable" {
            result.push((sid.to_string(), format!("User:{}", name)));
        }
    }
    result
}

/// "absent" (not loaded), "readable" (Software key opens), "unreadable" (no access).
pub fn service_profile_state(sid: &str) -> &'static str {
    let users = match open_subkey(RegRoot::Users, "", RegView::Default) {
        Some(key) => key.subkey_names(),
        None => return "absent",
    };
    if !users.iter().any(|name| name.eq_ignore_ascii_case(sid)) {
        return "absent";
    }
    match open_subkey(RegRoot::Users, &format!("{}\\Software", sid), RegView::Default) {
        Some(_) => "readable",
        None => "absent",
    }
}
