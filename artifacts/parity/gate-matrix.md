# Mich Startup Master 3.0.0 — Gate Matrix (final, live run)

| Flag | Exit | Verdict Snippet | Verdict |
|------|------|-----------------|---------|
| --version | 0 | MichStartupMaster 3.0.0 | PASS |
| --help | 0 | full CLI help (69 lines) | PASS |
| --smoke | 0 | SMOKE passed=true inventory=1066 invalid=0 | PASS |
| --list-sources | 0 | SOURCES count=32 | PASS |
| --ui-contract | 0 | responsive native control center contract | PASS |
| --ui-self-test | 0 | UI_SELF_TEST passed checks=13 passed=13 | PASS |
| --truth-self-test | 0 | TRUTH_SELF_TEST checks=15 passed=15 | PASS |
| --inventory-self-test | 0 | INVENTORY_SELF_TEST checks=13 passed=13 surfaces=31 | PASS |
| --managed-startup-dedupe-self-test | 0 | MANAGED_DEDUPE checks=4 passed=4 | PASS |
| --state-store-self-test | 0 | STATE_STORE workers=12 atomic=ok recovery=ok | PASS |
| --release-gate-self-test | 0 | RELEASE_GATE_SELF_TEST_OK passed=true | PASS |
| --agent-registration-self-test | 0 | AGENT_REGISTRATION_SELF_TEST_OK required=19 all true | PASS |
| --quiet-policy-probe | 0 | passed=true | PASS |
| --quiet-lineage-self-test | 0 | passed=true | PASS |
| --quiet-performance-probe | 0 | passed=true | PASS |
| --audit-boot | 0 | BOOT_AUDIT gaps=0 errors=0 + TRAY_AUDIT apps=3 running=3 findings=0 | PASS |
| --audit-tray | 0 | TRAY_AUDIT apps=3 running=3 findings=0 | PASS |
| --verify-live-inventory | 0 | LIVE_INVENTORY_VERIFY passed=true scanned=1066 | PASS |
| --detect-new | 0 | DETECT_NEW count=1 | PASS |
| --list-managed | 0 | managed tasks JSON (10 entries) | PASS |
| --export | 0 | header "Startup ID\tApplication\t" + rows | PASS |

**Result: 20/20 PASS. Zero failures.**
