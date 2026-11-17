# Mich Startup Master — Design System v3.1 (egui control center)

Direction: **Precision & Density** — a dev/admin startup control center that a
senior designer ships, not demos. Graphite-layered near-black surfaces, ONE
amber accent (#FFB454) for schedule times and focus, green/red strictly
semantic (on/off intent), Geist for prose, JetBrains Mono for anything
machine (commands, schedules, paths, counts). No gradients, no decoration
that doesn't inform, zero popups of any kind.

Stack: Rust + eframe/egui (custom-drawn rows), fonts bundled as TTF
(Geist Regular/Medium/SemiBold, JetBrains Mono Regular/Medium), native
Win32 tray on a dedicated thread. Window paints from the cached inventory
instantly (~750 ms to visible) and live-scans in the background.

## Semantic colors

| Role | Hex | Use | Contrast on #16161A |
|---|---|---|---|
| bg / canvas | `#16161A` | window background | — |
| surface | `#1D1D23` | odd rows, windows | surface, not text |
| surface-2 | `#26262E` | inputs, buttons, chips base | — |
| surface-3 | `#2C2C35` | hover fills | — |
| border | `#2F2F38` | hairlines (1px), toolbar dividers | — |
| selection | `#332D24` | selected row (surface + accent 16%) + 3px accent bar | — |
| accent | `#FFB454` | schedule chips, active sort, mixed state, undo | **9.4:1 AA+** |
| text | `#EDEDF2` | names, values | **15.2:1 AAA** |
| text-muted | `#8B8B98` | commands, sources, status bar | **5.9:1 AA** |
| good | `#3FB950` | on-state dot + toggle + Enable verb | 7.2:1 AA+ |
| danger | `#F85149` | off-state dot + Disable verb + High impact | 5.0:1 AA |

Rule: accent appears in the schedule chip, the active sort marker, the
selected-row bar, and the undo pill. The bulk verb is a **danger-ghost**
button (outline + intent color, never a filled primary). Everything else
stays gray-family so schedule times are the loudest information on screen.

## Type (Geist / JetBrains Mono, bundled)

| Token | Family | Size | Use |
|---|---|---|---|
| display | Geist SemiBold | 19px | dialog titles |
| name | Geist SemiBold | 14.5px | row names |
| body | Geist Regular | 14px | status messages, dialogs |
| button | Geist Medium | 13px | toolbar buttons, verbs |
| label | Geist Medium | 11.5px | column headers, status words, sources |
| chip | JetBrains Mono Medium | 12px | schedule chips |
| machine | JetBrains Mono Regular | 12px | entry commands, counts, paths |

## Spacing & shape

- 4px grid; space-1..8 = 4/8/12/16/20/24/32/40. Row rhythm 58px (group) +
  44px (expanded route sub-rows), toolbar 46px, header 30px, status 38px.
- Radius tokens: 4 (dots' inner elements) / 7 (rows, chips, buttons) / 10.
- Hairline dividers (1px border) separate brand / filters / actions zones.

## Row anatomy (left → right)

status dot (Ø6 + ring, halo when changed <12s) · name (semibold, elided) ·
entry command (mono, muted, elided) · status word "On / Off / Mixed · N"
(semibold, right-aligned) · **schedule chip** (mono, accent-on-tint,
min-width 104px) · source (medium, muted, clipped) · impact (right-aligned)
· **toggle switch** (38×21, animated knob 160ms). Grouped apps show a
chevron; expanding reveals per-route sub-rows (source badge, state, exact
registry/folder/task location, command, per-route mini toggle).

## Interaction laws

- One click on a switch = enable/disable, **works on every row, no
  exceptions** (authorization gates lifted at the UI layer; the underlying
  mutation keeps its exact snapshot + rollback contract).
- The bulk verb reflects the selection ("Disable · N"), danger-ghost styled.
- Selection always exists (first row on open); Enter toggles; arrows move;
  double-click edits; right-click opens the row menu (verb, Edit, Run now,
  Open location, Copy command/data, Quiet/Window, Delete, **Hide**,
  **Purge everywhere**).
- Every action is undoable for 15s from the status bar (Undo pill) — the app
  never uses confirm dialogs, balloons, or message boxes. Child processes
  (schtasks, PowerShell, cmd) always run with CREATE_NO_WINDOW.
- Hovering a row reveals the full truth: every route, its exact location,
  and its command.
- Column headers sort (Startup item / Status / Schedule / Source / Impact);
  the default **Smart** order is enabled-first, most-recently-changed first.
- Copy all / Ctrl+C copies the full 35-column TSV of every entry.
- Paste & add / Ctrl+V turns a copied program path into a startup entry
  automatically (name derived, runs exactly once per sign-in).
- Closing hides to the tray (agent + live monitor keep running); Quit lives
  on the tray menu.

## Elevation & DPI

- Borders-only ladder (bg → surface → surface-2); no shadows.
- PerMonitorV2 DPI (eframe scales the full UI); min window 940×660.
