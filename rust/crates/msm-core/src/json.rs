//! `--list` JSON with the exact C# field order and escaping.

use crate::model::StartupItem;

pub fn to_json(items: &[StartupItem]) -> String {
    let mut out = String::from("[");
    for (i, x) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let b = |v: bool| if v { "true" } else { "false" };
        out.push_str("{\"enabledEvidence\":\"configuration-only; consult startupState\",\"popupEvidence\":\"configuration-only; consult presentationState\"");
        out.push_str(&format!(",\"startupState\":\"{}\",\"startupReason\":\"{}\",\"presentationState\":\"{}\",\"presentationReason\":\"{}\"",
            esc(x.state_text()), esc(&x.evidence_reason), esc(&x.presentation_state), esc(&x.presentation_reason)));
        out.push_str(&format!(",\"id\":\"{}\",\"name\":\"{}\",\"appName\":\"{}\",\"applicationIdentity\":\"{}\",\"applicationRoot\":\"{}\",\"applicationIdentityReason\":\"{}\",\"runtimeEvidence\":\"{}\"",
            esc(&x.id), esc(&x.name), esc(x.human_name()), esc(&x.application_identity), esc(&x.application_root), esc(&x.application_identity_reason), esc(&x.runtime_evidence)));
        out.push_str(&format!(",\"source\":\"{}\",\"scope\":\"{}\",\"registryView\":\"{}\",\"location\":\"{}\",\"enabled\":{},\"canDisable\":{}",
            esc(&x.source), esc(&x.scope), esc(&x.registry_view), esc(&x.location), b(x.enabled), b(x.can_disable)));
        out.push_str(&format!(",\"mutationCapability\":\"{}\",\"mutationReason\":\"{}\",\"requiresExpertConfirmation\":{},\"requiresElevation\":{},\"requiresReboot\":{},\"externalAuthority\":\"{}\"",
            esc(&x.mutation_capability), esc(&x.mutation_reason), b(x.requires_expert_confirmation), b(x.requires_elevation), b(x.requires_reboot), esc(&x.external_authority)));
        out.push_str(&format!(",\"bootImpact\":\"{}\",\"bootImpactMs\":{},\"command\":\"{}\",\"status\":\"{}\",\"popup\":\"{}\",\"risk\":\"{}\",\"riskLabel\":\"{}\",\"riskReason\":\"{}\",\"advice\":\"{}\",\"adviceLabel\":\"{}\",\"adviceReason\":\"{}\"}}",
            x.boot_impact_label(), x.boot_impact_ms(), esc(&x.command), esc(&x.status), x.popup_label(), esc(x.risk_level()), esc(x.risk_label()), esc(x.risk_reason()), esc(x.advice_level()), esc(x.advice_label()), esc(&x.advice_reason())));
    }
    out.push(']');
    out
}

/// C# `Esc()` — escape every control character (JSON-strict).
pub fn esc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
