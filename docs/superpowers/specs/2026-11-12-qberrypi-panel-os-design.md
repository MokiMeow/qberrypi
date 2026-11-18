# qberrypi front panel as a small OS: HOME, five sections and the states of the system

Date: 2026-11-12. Status: design for the panel-os milestones (P1–P3). Binding beside
`2026-11-10-qberrypi-design.md`, `2026-11-10-qberrypi-cutover-and-api.md`,
`2026-11-11-qberrypi-watchdog-design.md` and `2026-11-11-qberrypi-only-os-design.md`; their hard
rules come first: **nothing may slow a cached, blocked or forwarded answer**, privacy, one static
binary, and the Pi writes nothing to the card in permanent mode outside an update.

**Source of truth for the look:** round 8 of the mockups, home "1 · Wall" (owner-approved), for P1 (P2 and P3 follow round 9's mix, §12):
`README.md`, `src/screens.js`, `src/homes.js` (the `wall` home and `systemDoors`), `src/parts.js`,
`src/stage.js`, `src/common.js`, `src/d6.js`, and the stills `r8-wall-<screen>@2x.png` (960×640,
nearest-neighbour 2× of the 480×320 panel). Where this spec and the JS disagree on a coordinate,
a colour or a word, the JS wins, except where this spec lists a deviation (§11). The mockups live
outside this repository; implementers are given the folder as `QBP_MOCKUP_DIR`.

## 0. Decisions

**Decided by the owner (2026-11-12):**

1. **ADS PASS** is the word for a device exempt from the ads lists, on every panel screen and in
   qnet's output. (The API's JSON keeps its field names, `exempt`, `exempt_macs`; only words a
   person reads change.)
2. **The lock is built and off until a PIN is set.** With a PIN, the strong actions (§7) ask for
   it first. Actions that only make the house stricter never ask.
3. **An arriving alert opens its own screen** from wherever the panel is (but a whole-system
   state, §5.4). The Drain's alert card is retired.
4. **Where the PIN lives (P3), as §7.2 recommended:** the durable PIN is an optional
   `[panel] pin_hash` line in the slot's `qberrypi.toml` (PBKDF2, the API password's format, made
   by `cargo xtask hash-pin`), so it survives a power cut with no card write. The desktop can set
   or clear it for the running boot through `PUT /api/qbp/panel`, and qnet pushes it again after
   a start, as it does the exempt list. A PIN set on the panel itself works at once but lasts
   until the next restart, and the LOCK screen says so.
5. **The gates are not PIN-locked**, as in the mockup: opening the main gate or a device's gate
   (any of 5, 30 or 60 minutes) never asks (§7.1); they shut by themselves within an hour.

## 1. Intent

The panel today is one scene, the Drain, with one key (pause blocking for five minutes) and an
alert card. Round 8 turns it into a small OS: a status lane on every screen, a HOME of six tiles,
and five sections (LIVE, DEVICES, ALERTS, NAMES, SYSTEM) plus the gate, with whole-screen states
for starting, updating, night and the PIN. The Drain survives unchanged as LIVE. The owner can do
from the screen what today needs the desktop: let ads pass on one device, open one device's gate
for a while, dismiss an alert, allow or refuse one name, set night hours, rejoin Wi-Fi, restart.

## 2. Inventory: what exists, what the Pi must gain

| Need | Exists today | Gains |
|---|---|---|
| Gate (main pause) | `BlockingControl::pause_for(secs)`, any length; panel taps 300 s only | Durations 5/30/60 min from the panel; the gate refill counts against the length chosen |
| Device list | `watch::devices::Devices::view()`: MAC, IPv4s, hostname, first/last seen, `exempt`, `blocked`, mode | Owner labels pushed from qnet (`PUT devices/labels`, memory only); a `rev` on the device lists |
| ADS PASS on one device | Pushed exempt list (list mode) and pushed block list (exempt-by-default mode), memory only, replaced whole by `PUT devices/exempt` / `devices/policy` | `Devices::set_ads_pass(mac, bool)`: edits the pushed list of the mode in force; bumps `rev`; qnet adopts (§8) |
| One device's gate | none | `Devices::open_gate(mac, secs)` / `shut_gate(mac)`: an in-memory map MAC → monotonic deadline; the device counts as exempt while open; expired by a timer in the device watch. **No DNS-side change** (§6.1) |
| "Asking now" lamp | `dns::clients::Clients` keeps each IPv4's last monotonic second (already updated per query) | The panel copies it once a second while HOME or DEVICES shows |
| A device's names | none (the query log has no client) | Names stream **only while that device's screen is open** (§6.2, §11) |
| Waiting alerts | `PendingAlerts::top()` and `ack(id)`; `AlertStore` with `count`, `first_unix`, `last_unix`, revisions | `PendingAlerts::waiting(n)` (newest first, unacked), `waiting_count()`, `last_alert_unix()` |
| Alert dismissal ↔ qnet | Already one store: the panel's `ack` is the API's `ack`, and `GET alerts?since=` reports it; qnet's acks remove the panel's tile at the next refresh | nothing new; a test that pins it |
| Owner's allow/refuse list | `dns.allow` / `dns.deny` in the config, built into the `BlockSet` | `qb_dns::OwnerNames` overlay in the `Resolver` (§6.3); `GET/PUT /api/qbp/names` (memory only, chunked like the known list); panel edits bump `rev` |
| Most refused today | `Stats` keeps 24 h hourly counts for 1,024 blocked names (LRU) | A per-name count for the local day and an incremental top 8, kept in `Stats::record` for blocked outcomes only (§6.4) |
| List size | `Snapshot::list_names` | nothing |
| Night hours | none | `[panel] night = "23-07"` in the config (default `23-07`); panel and API edit it for the boot; drawing only (§5.4) |
| Wi-Fi reading | `WifiStatus { associated, signal_dbm, ap, freq_mhz }` | The monotonic second the link joined, for `LINKED n D` |
| Wi-Fi rejoin | `wifi::run` rejoins only after a loss | A `Rejoin` notify that `hold` selects on: disconnect (reason 3), then a hinted pass; at most once a minute |
| Restart | `UpdateCtl::request_restart(Restart::Plain(Cause))`, init reboots | `Cause::Owner` (logged at INFO: "the owner restarted the board from the panel") |
| Update states | `UpdateCtl::view()`: phase Receiving / Trial(waiting_for, began) / last `Outcome` (confirmed / rolled back) | Bytes received so far (one `AtomicU64` in the receiver) for the receiving percentage; the trial's confirm deadline in the view |
| Check for an update | The Pi cannot fetch a build; updates are pushed by `cargo xtask update` | **CHECK NOW is dropped** (§11). VERSION shows the running build, its slot and the last update's outcome |
| Uptime, load, memory, heat | `status.rs` uptime; `thermal::read_c`; the unmerged `bench` branch adds `usage.rs` (cores' busy share, RSS) to `/api/v1/stats` | `crate::usage` lands with P3 as the one place both read (§6.5) |
| PIN | none | `panel::lock`: hash, verify, backoff (§7) |

## 3. The frame and the screens

All numbers are the README's "Numbers for building the wall" and the JS; palette `C` of
`common.js` (the Drain's `look.rs` already holds it); JetBrains Mono Regular and Bold from
`crates/qb-panel/fonts`; flat rects, circles, triangles and text; targets ≥ 48×48; taps only.

### 3.1 The lane (status bar), every screen but LIVE and the whole-system states

- Rails at y 4–8 and 64–68 (`G4`). Letters 24 px bold, one per cell, centre x `12 + 24·cell`,
  baseline 45.
- `HOME` in cells 0–3: white on HOME, `G2` elsewhere. Lit rail (white) under it on HOME
  (x 0–96), else under the section word (from x 120, `24·len` wide).
- Cells 5–11: on HOME the system in two words, `ALL WELL` (`G1`), `n ALERT(S)` (white), or
  `GATE OPEN` (white), in that order of precedence; elsewhere the section's word (`LIVE`,
  `DEVICES`, `ALERTS`, `NAMES`, `SYSTEM`, `GATE`), white.
- Waiting alerts: a white tile 22×40 at (313, 16) with a black 24 px digit (`+` above 9), on
  every non-HOME screen except under ALERTS.
- Clock `hh:mm` in cells 15–19, `G2`; `--:--` while the clock is not set.
- Targets: y < 72 and x < 108 → HOME; x ≥ 348 → SYSTEM; between → on HOME the door the two
  words name (ALERTS, LIVE, or `GATE OPEN` shuts the gate), elsewhere the section's root.

### 3.2 HOME (`homes.js` wall)

Six tiles 150×108 at `x 8 + 157·col, y 80 + 116·row`: label 16 px at +10,+27 (tracking 2), lamp
r 5 at right −16, +21, figure 40 px at +8,+72, line 16 px at +10,+96 (tracking 1).

| Tile | Figure | Line | Lamp | Door |
|---|---|---|---|---|
| LIVE | refused today (`format::count`) | `44% REFUSED` (`Share`) | white for 3 frames after a lane name, else blue | LIVE |
| DEVICES | devices on the wall | `n BLOCKED`; mini wall 6×2 at +66,+45 (10×10 on a 13 px pitch: blue blocked, hollow `G2` ads pass or open, white asking, dot empty) | white while one asks | DEVICES |
| ALERTS | waiting count | `NEWEST hh:mm` or `ALL CLEAR`; white tile while any wait, hollow `G4` when none | | ALERTS |
| GATE (a key) | `SHUT` or `m:ss` | `ADS REFUSED` / `ADS PASS`; blue block shut, hollow white open (label `OPEN`) | none | shut: GATE screen; open: shuts at once |
| NAMES | list size (`212K`) | `NAMES LISTED` | | NAMES |
| SYSTEM | version | `UP n DAYS` | | SYSTEM |

A figure the Pi does not know yet is `—` in `G3`.

### 3.3 LIVE

The Drain exactly as built, with x 0–96, y 8–64 cleared and `HOME` drawn in cells 0–3 (`G2`).
Names leave the lane at cell 4 instead of the left edge (the mockup's open question 3, accepted
with the design). The alert card is retired: an alert opens ALERT (§5.3), and the Drain's
`Stills::alert` is removed. Taps: the key, shut → the gate's choices unfold in place over the
field (`drawGateAsk`: x 120–480, y 80–320; keys 110×80 at `128 + 117·i`, 224; a tap anywhere else
closes it; they close by themselves after 8 s); the key, open → shut. The share and the clock
column: share (y < 148) → NAMES, clock → SYSTEM; the funnel and the count (x ≥ 120) → NAMES;
lane x < 108 → HOME.

### 3.4 DEVICES

- **Wall:** 12 places, 110×70 at `x 8 + 118·col, y 80 + 77·row`. A device: name (label, else
  hostname, else `·<last octet>` in 24 px) in up to two 16 px lines (baselines +22, +40), state
  word 16 px at +61: `BLOCKED` (blue tile, word `BLUE_NIGHT`), `ADS PASS` (hollow `G3`, word `G2`),
  `OPEN m:ss` (hollow white, word white), `ALWAYS` (router and the Pi). Lamp at the top right:
  r 5–6 white while asking, else r 2.5. The first empty place says `NEW ONES` / the default for a
  newcomer in the mode in force (`ADS PASS` or `BLOCKED`), `G3`.
- **Which devices, in what order:** devices with an owner label in qnet's label order, then the
  others seen in the last 24 h by first sighting, then the router and the Pi. The order is stable
  so a tile never moves while the screen shows. Past 11, the twelfth place is `MORE · n` and turns
  the page (pages of 11).
- **DEVICE:** name 40 px (30 px past 15 letters) at (6, 116), `·octet` right at 472 in `G2`; two
  rows of names (24 px, 52 px pitch from y 132) with a 16 px square and `REFUSED / PASSED /
  ANSWERED` right; the floor: left the state key (`ON THIS DEVICE` / `ADS BLOCKED` blue,
  `ADS PASS` hollow, `ALWAYS BLOCKED` `BLUE_DK`); right `ITS OWN GATE / OPEN A WHILE` (blocked),
  the open key with `m:ss` (open; a tap shuts it), or `ALREADY OPEN` / `NEVER OPENS` in `G3`.
  A name row opens NAME (P2).
- **CONFIRM** (`flipScreen`): slab drawn as the state it would take, `LET ADS PASS?` hollow or
  `BLOCK ADS?` blue; lines: the device and `Ads pass now.` / `Ads are blocked now.` + ` Threats
  stay blocked.`; floor `KEEP ADS PASS` / `KEEP BLOCKED` and `ALLOW ADS` (strong, padlock when
  locked) / `BLOCK ADS` (never asks).
- **ITS GATE** (`pauseScreen`): slab `OPEN ITS GATE`, the device, `Ads pass on this device for a
  while.`; four keys: `KEEP / SHUT`, `5 MIN`, `30 MIN`, `1 HOUR`.

### 3.5 GATE (from HOME)

Slab `OPEN THE GATE`, `Ads pass on every device.`, `Threats stay blocked. It shuts itself.`; the
same four keys. The main gate is not a strong action (decided with the design; §7.1).

### 3.6 ALERTS (P2)

The newest three waiting as slabs 464×68 at y `80 + 78·k`: white for High, blue otherwise;
title 24 px, time right (`hh:mm`, or the weekday past today), subject and who 16 px. None
waiting: `NOTHING TO REPORT` letter-spaced in `G2` and `LAST ALERT <day> hh:mm` in `G3` (nothing
when the store never had one). **ALERT:** the slab 464×152 in the alert's colour with the
title and up to three lines (§5.3), floor `DISMISS` (acks, back to ALERTS) and `SEE DEVICE`
(the device whose IPv4 or MAC the alert names; `G3` and inert when none is on the wall).

### 3.7 NAMES (P2)

`MOST REFUSED TODAY` (16 px `G2`) and the door key `YOUR LIST n ▸` (200×48 at 272, 80); three
bars 52 tall at `y 136 + 56·k`, width by count (≥ 40), `BLUE`, `BLUE_MD`, `BLUE_DK`; name 24 px
(`fit_site`, 22 cells) and count right. **YOUR LIST:** the newest three of the owner's list,
refused solid blue, allowed hollow white, `REFUSED` / `ALLOWED` and the day; `nothing of your own
yet`. **NAME:** slab (blue while refused), the name (`fit_site`, 24 cells), line one `refused
1,204 times today` / `answered …` / `refused by you since Mon`, line two which list holds it (§11);
floor `KEEP REFUSED` + `ALLOW NAME` (strong) or `KEEP ANSWERED` + `REFUSE NAME` (never asks). A
name on a threat list: right key `THREAT · STAYS` in `G3`, inert. An owner entry can be taken
back: on an owner-allowed name the right key is `REFUSE AGAIN` (removes the entry), on an
owner-refused one `ANSWER AGAIN` (strong: it loosens).

### 3.8 SYSTEM (P3)

The wall again (`systemDoors`): VERSION (`0.4.1`, `UP TO DATE` → the last outcome word), NIGHT
(`23–07`, `DARK HOURS`; `OFF` when none), WI-FI (`GOOD` / `FAIR` / `WEAK` / `DOWN`, `5 GHZ LINK`),
UP (`12 D`, `LOAD 0.21`), LOCK (`ON` / `OFF`, `PIN IS SET` / `NO PIN SET`), ABOUT (`3B+`,
`QBERRYPI`). Each opens a reading screen (`reading()`): label 16 px at baseline 104, figure 64 px
at 170, lines 18 px at 202 and 225, floor keys.

| Reading | Figure | Lines | Floor |
|---|---|---|---|
| VERSION | `0.4.1` | `built 2026-11-12 · slot A` · `last update: confirmed Mon 09:00` / `rolled back …` / `none since start` | `BACK` only |
| NIGHT HOURS | `DARK FROM hh:00` / `UNTIL hh:00` with `−` `+` keys 80×64 | — | `SLEEP NOW`, `DONE` |
| WI-FI | `GOOD` + five bars | `5 GHz · signal −54 dBm · linked 12 days` · `upstream answers in 9 ms` | `BACK`, `REJOIN` |
| UP | `12 D 4 H` | `load 0.21 · memory 38% · 47 °C` · `last restart: <cause>` | `BACK`, `RESTART` (strong) |
| LOCK | `ON` / `OFF` | the four strong actions · where the PIN lives (§7.2) | `KEEP OFF` + `SET A PIN`, or `KEEP ON` + `TURN OFF` (strong) |
| ABOUT | `QBERRYPI` | `a DNS gate and watchdog for the house` · `version … on a Pi 3B+ · panel 480×320` | `BACK` |

`RESTART` and `REJOIN` are inert (`G3`) while an update is received or on trial.

## 4. Navigation and taps

- **One root and a stack.** `Nav { root: Section, stack: [Screen; ≤ 6] }`, as `stage.js`:
  `go(section)` replaces the stack; `open(screen)` pushes; `back()` pops. The deepest path is
  ALERTS → ALERT → DEVICE → CONFIRM → PIN (5). A push past the bound replaces the top. Screens
  carry their argument by identity, never by index: a device by MAC, an alert by store id, a
  name by its text. When the thing is gone (an alert acked on the desktop, a device forgotten),
  the screen pops itself at the next refresh.
- **Three places that never move:** the lane's HOME, the section word (back to the root), and the
  floor's left key (keep things as they are: back).
- **Floor** is y ≥ 236; left key x < 240 (four keys: `floor((x − 4) / 118)`).
- **Home by itself** after 60 s without a touch, from any screen but HOME and the whole-system
  states; the PIN pad is timed out too (to where it came from, the action not done).
- **A tap** is a press and release on the same target, ≥ 50 ms (`TapTracker` today); the target
  is decided at the press (as now). The tracker becomes generic: `TapTracker` yields `(x, y)` of
  the press, and `nav::tap(&Nav, &World, x, y) -> Tap` decides.
- `Tap` is `None`, `Nav(change)`, or `Act(Intent)`. An `Intent` is what the system must do:

```rust
enum Intent {
    Gate(GateMove),                 // Open(secs) | Shut
    DeviceAds { mac: Mac, pass: bool },
    DeviceGate { mac: Mac, secs: u16 },   // 0 shuts
    Ack(u64),
    Name { name: String, verdict: Verdict },   // Allow | Refuse | Clear
    NightHours { from: u8, to: u8 },
    SleepNow, Rejoin, Restart,
    SetPin(PinDigits), ClearPin,
}
```

  `Intent::is_strong()` is the one list of §7.1. qb-panel returns intents; qberrypi carries
  them out. A strong intent with the lock on opens the PIN state with the intent held in it; the
  fourth digit asks qberrypi to verify (§7.3).

## 5. Architecture

### 5.1 qb-panel (no I/O, as now)

```
crates/qb-panel/src/
  drain/            the Drain, unchanged but: no alert card; HOME cells; gate ask in place
  ui/look.rs        the palette (moved from drain/look.rs, re-exported there) and the OS's numbers
  ui/canvas.rs      rect, hollow, dot (any radius; analytic coverage, 4×4 samples), tri, text set
                    glyph by glyph with tracking and anchor (the mockup's tx/txr/laneSay)
  ui/painter.rs     ScreenPainter: paints a screen whole into its RGBA frame when its View changed,
                    converts to RGB565, and reports the changed rows
  os/world.rs       World: plain data the screens read (§5.2)
  os/nav.rs         Section, Screen, Whole, Nav, Tap, Intent; tap routing
  os/screens/*.rs   one file per family: lane, home, live (glue), devices, gate, alerts, names,
                    system, whole (starting, update, night, pin)
  os/fixture.rs     the mockup's house, alerts and names as a World; one fixture per still
```

- **One painter per kind.** LIVE keeps the Drain `Painter` (its dirty-row logic and budget
  tests stay). Every other screen uses the `ScreenPainter`: build the screen's `View` (a small
  `PartialEq` struct of exactly what it draws, words already fitted), and only if it differs from
  the last one paint the whole frame in memory (≈ 3 ms on the Pi: measure) and diff the RGB565
  rows. The write is the span of changed rows, as fbtft sends it.
- **The Drain runs whatever shows.** Lane names are taken and offered every frame, and the
  Drain ticks while moving, so LIVE is current the moment it opens. It is painted only on LIVE.
- **Switching screens** is one full write (allowed, like the pause toggle today). The glass's
  last frame is kept by the panel loop, so the first write after a switch is diffed against it.
- **Budget:** every write within a screen ≤ 80 rows (`changed_rows`); the allowed one-off large
  writes are a screen change, a whole-system state change and Wi-Fi joining. HOME's lamps, the
  gate countdown, the clock and the alert count are separate small writes (a lamp is 11 rows).

### 5.2 What the panel reads, and how often

The panel thread is a `Component::blocking` on the pool (as now) and never runs on the runtime
thread. It assembles a `World` from these reads, each copying out under its own lock:

| Read | Lock taken | When |
|---|---|---|
| `Source::snapshot()` (counts, Wi-Fi, clock, lists, upstream ms) | the resolver's, for a copy of the counts (as now) | 1 s |
| Blocking control | the resolver's, for `paused_until` (as now) | every frame (as now) |
| Lane names | the lane ring (`try_lock` on the DNS side; as now) | every frame |
| Device inventory view + gates | the inventory's (`Mutex<Devices>`), never the resolver's | 2 s, only while HOME, DEVICES or an alert's device shows |
| Client activity (last second per IPv4) | `Dns::clients` (already locked per query for one insert) | 1 s, only on HOME / DEVICES |
| Watched device's names | the watch ring (`try_lock` on the DNS side) | every frame, only on DEVICE |
| Waiting alerts | the store's own | 1 s |
| Top refused today + owner list | the resolver's, for a copy of ≤ 8 + ≤ 3 entries (no scan) | 5 s, only on NAMES / NAME |
| System readings | none: `/proc`, `/sys` on the panel thread | 5 s, only on SYSTEM / UP |
| Update view | `UpdateCtl`'s own | 1 s |

So the resolver lock is taken by the panel for copies only, as today, once a second, plus once
in five seconds on NAMES; never for a scan. Nothing the panel does adds work to the answer path
but what §6 lists and measures.

### 5.3 Alerts on the panel

- A new waiting alert (an id the panel has not shown) opens ALERTS → ALERT from any screen but
  the whole-system states; the newest High first if several came in one refresh, the others wait.
  Shown once: idle-home or `back` leaves it waiting (white ALERTS tile, lane count). At night it
  does not light the screen (§5.4).
- The ALERT lines are made from the store's fields per kind, in qberrypi (`panel/view.rs`):
  `threat`: the name; `<device> asked n times by hh:mm`; `on the threat lists · never answered`.
  `new_device`: `<device> joined at hh:mm`; `no name yet · <default word> by default`.
  `arp_spoof`: `<device> answers as router`; `n times since hh:mm`. Others: the subject and
  `n times since hh:mm`. `<device>` is the label, hostname or `·octet`. The subject is fitted on
  the panel thread and never logged (as `AlertCard` today: no `Debug` that prints it).
- **Dismissal and qnet:** `DISMISS` is `AlertStore::ack(id)`, the same call `POST alerts/{id}/ack`
  makes, so qnet sees the ack through `GET alerts?since=` and an ack from qnet takes the alert off
  the panel within a second. Nothing new on the wire.

### 5.4 Whole-system states

They take the whole screen, draw their own lane (`laneMid`), and offer no HOME.

- **STARTING:** from the panel's first frame until DNS answers and 3 s have passed, or 90 s:
  five lines `PANEL ON`, `WI-FI JOINED`, `CLOCK SET`, `LISTS 212K NAMES`, `DNS ANSWERING` (lamp
  r 8 at x 20: blue done, white under way, dot to come). Readings: the panel up; `wifi.associated`;
  `clock_set`; `list_names`; the slot filled and `today.queries > 0` or the self-test passed. Then
  HOME. It replaces the Drain's `JOINING WI-FI` boot look (which LIVE keeps after a later drop).
- **UPDATE:** `RECEIVING UPDATE` (the funnel fills by percent; `DNS STILL ON`; the version left),
  `TRIAL CONFIRMS IN` (`m:ss` to the trial's deadline, the funnel running out), `UPDATE
  CONFIRMED` / `UPDATE ROLLED BACK` (a tap or 30 s → HOME). From `UpdateCtl::view()`. A trial boot
  shows the trial screen once STARTING is done.
- **NIGHT:** in the night hours (and after `SLEEP NOW` until a touch), the clock 96 px in
  `BLUE_DEEP` at (96, 214) and one falling drop. A touch wakes to HOME; 60 s idle sleeps again.
  A waiting alert turns the drop white; an arriving one does not light the screen. Drawing only:
  the piscreen's backlight is not switched (whether it can be is a Pi check in P3; if a GPIO
  drives it, turning it off is a follow-up).
- **PIN:** §7.

## 6. The Pi's new parts, and the hot path

The rule: no cached, blocked or forwarded answer gets slower. Each change states its cost on the
answer path; the gate for each is the existing ignored benches (`decide_cost_*`,
`observe_costs_little`, run `--release -- --ignored --nocapture`) before and after, and on the Pi
`cargo xtask latency` before and after the milestone (the owner runs it; p50 and p99 of cached,
blocked and forwarded must not move beyond the run-to-run spread).

### 6.1 Device gates and ADS PASS edits: zero answer-path cost

`Devices` gains `gates: HashMap<Mac, u64>` (monotonic deadline, ≤ `MAX_EXEMPT` entries) and
`set_ads_pass(mac, pass)`. A device is exempt while its gate is open, in either mode; the
inventory republishes `Exemption::listed` / `blocked` exactly as for a list change, so the DNS
side reads the same two sets it reads now. The device watch's loop gains a `sleep_until(next
gate deadline)` arm that expires gates (its 30 s ticker is too coarse). `set_ads_pass` edits the
pushed exempt list in list mode and the pushed block list in exempt-by-default mode; a device on
the configuration's list is `fixed` (its key says `SET IN CONFIG`, inert). Router and Pi stay
never-exempt. Each edit bumps the inventory's `rev`. Gates are never pushed back by qnet:
they are temporary and die with a restart.

### 6.2 A device's names: one relaxed load per query

The mockup shows the last two names a device asked. Keeping that for every device would store
which device asked what, which nothing on the Pi does today. Instead `LaneFeed` gains
`watched: AtomicU32` (an IPv4, 0 for none) and a two-slot ring. `offer_latest` gets the client:
when `watched` is 0 (always, unless DEVICE is open) the cost is one relaxed load; when it matches,
the name goes into the ring by `try_lock` as lane names do (fixed 32-byte slots, no allocation).
The panel sets `watched` when DEVICE opens and clears it when it closes; the screen shows `waiting
for its next name` until one comes. Nothing is kept per device when no one is looking (§11).

### 6.3 The owner's names: one length check per query when empty

`qb_dns::OwnerNames { allow, refuse }`: two sorted `Vec<Box<str>>`, ≤ 128 names each, matched
like the configuration's `dns.allow` / `dns.deny` (the name and its parents). In
`Resolver::decide` step 4: `refuse` blocks for every client, including ADS PASS devices, counted
as blocked, never a threat and never an alert; `allow` lifts an ads-list or deny block; a name on
a threat list cannot be allowed (threats stay blocked). Both empty: two `is_empty` checks. Not
empty: a binary search per label suffix (≤ 4 for ordinary names). It is a field beside the
`BlockSet`, so a list refresh keeps it. The panel and the API replace it through
`Resolver::set_owner_names` under the resolver lock, built before the lock is taken (as
`replace_blockset`). Memory only; `GET/PUT /api/qbp/names` (§8).

### 6.4 Most refused today: O(1) per blocked answer

`Stats` gains, per tracked blocked name (the existing LRU entry), `day: i64, today: u32`, and a
`top: [(Box<str>, u32); 8]` for the local day: on a blocked record, the name's `today` is bumped
and, if it is in `top` or beats `top[7]`, `top` is updated (≤ 8 compares; a `Box<str>` clone only
when a name enters the top, which the existing per-block key already allocates anyway). `roll_day`
clears `top`. `Stats::top_today()` copies it. Blocked answers never touch the network, so the
added tens of nanoseconds are inside the blocked path's noise; the bench proves it.

### 6.5 Load, memory, heat: off the answer path

`crate::usage` (P3) reads `/proc/uptime`, `/proc/loadavg`, `/proc/meminfo` (`MemAvailable`) and
`thermal::read_c`, on the panel thread every 5 s while SYSTEM or UP shows. **Sharing with the
bench branch:** that branch's `usage.rs` (cores' busy share and RSS, sampled by the supervisor's
5 s tick for `/api/v1/stats`) and this one become one module: whichever merges second rebases onto
the first, `Usage` keeps its samples behind its own lock, and the panel reads `Usage::figures()`
plus load and `MemAvailable`. The `/proc/stat` parsing moves from `qb-bench` into
`qberrypi::usage` (qb-bench depends on it, not the other way round), so the binary does not grow
a dependency on the bench crate.

### 6.6 Wi-Fi rejoin, restart, update progress

- `Rejoin`: an `Arc<tokio::sync::Notify>` given to `wifi::run`; `hold` selects on it, disconnects
  with reason 3 and ends the pass as a held link (a hinted pass follows). The panel refuses a
  second rejoin within 60 s (`REJOIN` shows `WAIT`). The house loses DNS for the join's few
  seconds; the screen says `rejoining…` with the lane `JOINING WI-FI` as LIVE has it.
- `Restart`: `Cause::Owner`, through `request_restart`; the supervisor's normal path (log saved,
  board rebooted). Refused while an update is received or on trial.
- Receiving progress: `Receiving` counts bytes into an `AtomicU64`; `UpdateCtl::view()` gains
  `received` and the manifest's length.

### 6.7 Memory budget

Drain painter ≈ 1.2 MB (RGBA 614,400 + RGB565 307,200 + the compare buffer), ScreenPainter ≈ 1.2
MB, glyph caches ≤ 2 MB (16–96 px, two weights, printable ASCII, rendered on first use; the 96 px
night clock only needs digits and `:`), World ≤ 64 KB, owner names ≤ 2 × 128 × 253 B, top ≤ 8
names, labels ≤ 64 × 24 B, watch ring 64 B. Ceiling: **≤ 5 MB** more resident than today.
The Pi has no shell, so VmRSS is read from `/api/v1/stats` once `usage` reports it (P3, or the
bench branch), and before that from the QEMU self-test boot, which logs it (the panel is absent
there, so the painters are measured by their sizes in a unit test).

## 7. Security

Anyone at the screen can act; the panel is in the house, and the lock is the owner's choice.

### 7.1 Strong actions (ask for the PIN when the lock is on)

1. `ALLOW ADS` on a device (permanent ADS PASS).
2. `ALLOW NAME`, and `ANSWER AGAIN` on an owner-refused name.
3. `RESTART`.
4. `TURN OFF` the lock, and setting a new PIN while one is set.

Never asked: anything that makes the house stricter (`BLOCK ADS`, `REFUSE NAME`, shutting a
gate), reading any screen, dismissing an alert, opening the main gate or a device's gate (they
shut by themselves within an hour), night hours, `REJOIN`. A strong key wears the padlock
(`lockMark`) while the lock is on.

### 7.2 Where the PIN lives (decided, §0.4)

- **Durable:** `[panel] pin_hash = "$pbkdf2-sha256$i=10000$<salt>$<hash>"` in the slot's config,
  made by `cargo xtask hash-pin` (the `hash-password` code with 10,000 rounds and four digits).
  It survives a power cut, and the Pi writes nothing. Changing it rides `cargo xtask update`.
- **For this boot, from the desktop:** `PUT /api/qbp/panel {"pin_hash": "…" | null}` (behind the
  login). qnet keeps the hash and pushes it again after each start, as it pushes the exempt list.
- **On the panel:** `SET A PIN` works at once and is held for this boot only; the LOCK screen's
  second line says `set here: until the next restart`, and `GET /api/qbp/panel` reports
  `"lock": {"on": true, "source": "panel"}` so qnet can say it will not survive. `TURN OFF` on
  the panel lasts until the next restart when a config hash exists.
- The latest set wins for the boot; at a start, the config's hash (or none).
- The hash is never sent out: `GET panel` says only `on` and `source`. The PIN is four digits, so
  the hash is no protection against someone holding the card (the config is private anyway); its
  job is that no PIN is stored or logged in clear.

### 7.3 Handling

- Digits are held in a `Zeroizing<[u8; 4]>` from the pad to the verify; `PinDigits` has no
  `Debug` that prints them; no log line carries a digit or the hash.
- Verify with `api::password::PasswordHash::verify` (PBKDF2, constant-time compare), on the
  panel thread (10,000 rounds: measure on the Pi, budget ≤ 100 ms; the Drain is not shown then).
- **Backoff:** three wrong in a row → 30 s wait, then each further wrong doubles it, to 15 min at
  most; a right PIN resets. The pad shows `WAIT 0:30` and takes no digits. Logged: `panel: wrong
  PIN (n in a row)` at WARN, the count only. The counter is in memory; a power cycle resets it
  (worst case ≈ 3 guesses per boot, about a day for all 10,000; noted, accepted for a household
  lock).
- The pad times out after 60 s idle like any screen; the held intent is dropped.

## 8. API and qnet

New endpoints under `/api/qbp/` (behind the login, same rules: 4 KiB bodies, a bad body is a
400 and changes nothing, no answer repeats a piece of it):

| Method and path | Body / answer | Phase |
|---|---|---|
| `GET devices` (grows) | adds `rev`, `exempt_pushed`, `block_pushed`, `own_mac` (the Pi's MAC as text, or null), per device `label`, `label_rank` (number or null), `gate_left_secs`, `fixed`, `always` | P1 |
| `PUT devices/labels` | `{"labels": [["02:00:00:00:00:01", "Desk PC"], …], "append", "complete"}` (chunked as `known`), ≤ 64, label ≤ 24 printable chars | P1 |
| `PUT devices/exempt`, `devices/policy` (grow) | optional `"if_rev": n` → 409 `{"rev": current}` when the lists changed since | P1 |
| `PUT devices/gate` | `{"mac", "secs"}`, secs ≤ 3600, 0 shuts | P1 |
| `GET names` | `{"rev", "allow": [...], "refuse": [...], "top_today": [{"name","count"}]}` | P2 |
| `PUT names` | `{"allow", "refuse", "append", "complete", "if_rev"}`, ≤ 128 each, names validated as `dns.deny` | P2 |
| `GET panel` | `{"night": "23-07" | null, "lock": {"on", "source"}}` | P3 |
| `PUT panel` | `{"night"?, "pin_hash"?}` | P3 |

**Sync rule (what qnet must do; qnet's change is outside this repository):** the panel edits the
same in-memory lists qnet pushes. qnet reads `GET devices` / `GET names` on its usual poll; when
`rev` moved and the pushed lists differ from its own, it adopts the Pi's (the panel was the last
writer). It pushes with `if_rev` and, on 409, reads again and merges. After a Pi restart the
lists are empty and qnet pushes its own, which by then hold the panel's edits it has seen. A panel
edit made in the last poll interval before a power cut is lost; the panel says nothing of that
(accepted). qnet also adopts the word **ADS PASS** for exempt devices and shows open gates.

## 9. Testing

- **Reference images per screen** (`crates/qb-panel/tests/os_golden.rs`, references in
  `tests/reference/os-<still>.png`): one per wall still of the mockup, same names (`home`,
  `home-alert`, `home-open`, `gate`, `live`, `live-gate`, `devices` (ADS PASS words), `device`,
  `confirm`, `pause`, `alerts`, `alerts-none`, `alert`, `names`, `yours`, `name`, `name-locked`,
  `pin`, `system`, `version`, `nighthours`, `wifi`, `uptime`, `lock`, `about`, `boot`, `night`,
  `update-receiving`, `update-trial`, `update-confirmed`, `update-rolledback`), from `os/fixture.rs`
  worlds that mirror `stage.js`'s `SCENES` and `app.still` (devices 1 and 4 asking, lamp beat 0).
  The comparison is `drain_golden.rs`'s (mean, outlier share, worst tile), with `QBP_BLESS=1` to
  write and `QBP_MOCKUP_DIR` to compare regions with the round 8 stills (2× nearest). The README
  says which regions are expected to differ (CHECK NOW gone, §11).
- **Budgets** (`tests/os_budget.rs`): HOME, DEVICES and ALERTS run 150 s of the mockup's traffic
  at 8 fps with lamps, the clock, a gate countdown and an alert arriving: every in-screen write ≤
  80 rows; the screen changes counted apart. `drain_budget.rs` stays as it is.
- **Tap routing** (`os/nav.rs` tests): the map of the README as a table, every door to where it
  leads; the lane's three targets on every screen; the floor keys; strong intents become the PIN
  state only when the lock is on; idle 60 s → HOME from every screen but the whole-system states;
  an alert arriving opens ALERT, not during PIN / UPDATE / NIGHT; a screen whose thing is gone
  pops; a tap in a whole-system state does nothing but its own keys.
- **qberrypi panel loop** (`panel/mod.rs` tests with the `Recorder`): intents reach fakes of the
  controls; the PIN flow (right, wrong, backoff, timeout) with a fake verifier; the panel never
  logs a name, a label or a digit (`the_panel_never_logs_a_name` grows); reads happen only on the
  screens §5.2 lists.
- **Pi side:** unit tests for device gates (expiry republishes the sets), `set_ads_pass` in both
  modes, `rev` and `if_rev`, labels; `OwnerNames` in `qb-dns` (allow lifts ads, never threats;
  refuse blocks ADS PASS devices too; parents match); `top_today` (ordering, day roll, eviction);
  the watched-client ring; rejoin and restart causes; the API routes (shapes, 400/404/405/409).
- **QEMU** (`cargo xtask qemu-test`; the guest has no screen and no network card, so the panel is
  absent there and the API is reached only by the in-guest self-test, `selftest.rs` under
  `qberrypi.selftest.dns`): P1's self-test logs in over loopback and checks `GET
  /api/qbp/devices` answers with a `rev`; P2's refuses a made-up `.invalid` name with `PUT
  /api/qbp/names`, asks it over loopback and expects the blocked answer, then clears it; P3's
  reads `GET /api/qbp/panel`. Each adds one serial line that `dns_selftest_conditions` requires.
  Every existing scenario passes unchanged (boot time, warnings).
- **On the Pi** (the owner, after each `cargo xtask update`): a checklist per milestone in the
  plan (each screen by touch, the gate durations, a device's ADS PASS from the panel then seen in
  qnet, `cargo xtask latency` before and after, VmRSS before and after). `cargo xtask
  panel-preview --screen <still>` renders any still to a PNG for review without the Pi.

## 10. Phasing (each ships alone with `cargo xtask update`)

- **P1, the frame and the most-used parts:** the lane, HOME, GATE with 5/30/60, LIVE (Drain with
  HOME cells and the gate ask; card retired), the ALERTS tile and the lane count opening the top
  alert's ALERT screen with `DISMISS` (today's `top()`, so dismissing on the panel never goes
  away; the list, `SEE DEVICE` and the arrival rule are P2), DEVICES
  with ADS PASS, DEVICE, CONFIRM, ITS GATE, device gates, labels, `rev`, the watched names, the
  SYSTEM tile showing the version (opening an ABOUT reading only). The lock does not exist yet,
  so P1's strong actions are unguarded, as the Drain's key is today.
- **P2, ALERTS and NAMES:** ALERTS, ALERT (and the arrival rule), NAMES, YOUR LIST, NAME, owner
  names in qb-dns, top today, `GET/PUT names`.
- **P3, SYSTEM, the states and the lock:** SYSTEM and its six readings, STARTING, UPDATE ×4,
  NIGHT, PIN, the lock, `usage`, rejoin, restart, update progress, `GET/PUT panel`,
  `hash-pin`.

## 11. Deviations from the mockup (for the owner to see)

1. **CHECK NOW is dropped.** The Pi cannot fetch a build; an update is pushed from the desktop.
   VERSION shows what runs, its slot and the last update's outcome, with BACK only.
2. **A device's names are shown while its screen is open**, from that moment, not "the last two"
   from before: keeping per-device history would make the Pi record which device asked what,
   which it does not do today (privacy outranks the nicer screen).
3. **NAME says which list refused it, not "mostly asked by <device>"**, for the same reason; the
   NAME → DEVICE door goes. The bars' counts are exact for the 1,024 most recently blocked names.
4. **WI-FI's second line is the upstream's answer time**, which the Pi measures, not the router's.
5. **YOUR LIST shows the newest three** (no scrolling: taps only); the rest are in qnet.
6. **Night does not switch the backlight** until a P3 check shows it can be switched.

## 12. Phases 2–3 look (round 9 mix)

Decided by the owner (2026-11-17): P2 and P3 are drawn as round 9's recommended mix, `?pick=ACCCA`
(alerts A, names C, system C, update C, the other states A). The round 9 folder (`README.md`,
`src/kit.js`, `src/ca.js`, `src/cc.js`, round 8's harness in `src/`, stills
`r9-<A|C>-<screen>@2x.png`) is the source of truth for coordinates and words of these screens, as
round 8 is for P1; implementers get it as `QBP_MOCKUP9_DIR`. Every rule in §§3–9 still holds where
round 9 says nothing; where round 9's drawing and this section disagree, this section wins.

| Screen | Drawn as | Round 9 source |
|---|---|---|
| ALERTS, ALERT, nothing to report | A: the watch log on tractor-feed paper (newest printed last, at the bottom, over `hh:mm  WATCH CONTINUES` and the cursor; `n ON THE LOG`); an alert is its slip torn off, stamped | `ca.js` `alerts`, `alert` |
| PIN | A: the authorisation slip (`ENTER THE PIN` / `CHOOSE A PIN` / `WRONG PIN` / `WAIT m:ss`, four boxes, `FOR` what is signed for, `THE OWNER`) beside round 8's twelve keys | `ca.js` `pin`, `kit.js` `pinPad9` |
| STARTING | A: the ticked checklist (`PANEL ON` … `DNS ANSWERING`) | `ca.js` `boot` |
| NIGHT | A: the paper's margins in `BLUE_NIGHT`, the hour 96 px `BLUE_DEEP`, `NIGHT WATCH` | `ca.js` `night` |
| NAMES, YOUR LIST, NAME | C: the manifold (a pipe a name, thickness by count, valve shut blue / open white, a threat sealed with a plate); `YOUR OWN n ▸` / `VALVES YOU SET`; one name as a mimic | `cc.js` `names`, `yours`, `name`, `kit.js` `nameFacts`, `keys9`, `listDoor9` |
| SYSTEM | C: the one-line mimic (router, Wi-Fi link, the box with its two slots and readings), `NIGHT` / `LOCK` / `ABOUT` keys under it | `cc.js` `system` |
| WI-FI | C: the link drawn large, `REJOIN` | `cc.js` `wifi` |
| UPDATE ×4 | C: the changeover, two slots and DNS drawn unbroken | `cc.js` `update` |
| VERSION, UP, NIGHT HOURS, LOCK, ABOUT | round 8's plain readings (§3.8), until the owner asks otherwise | round 8 `screens.js` `reading` |

The floor keys keep each concept's dress: stubs under a perforation on A's screens (`keys9(.., 'A')`),
a thin outline on C's (`'C'`). §3.6, §3.7 and §3.8's SYSTEM wall are replaced by the above; §3.8's
reading table holds for the five plain readings.

**Where the build differs from the round 9 drawing, and why:**

1. **No device on a name (privacy, §11.3).** C's NAME starts its pipe at a device node and A's
   manifest shows `ASKED MOSTLY BY`; the Pi keeps no who-asked-what record and gains none. NAME's
   left node reads `THE HOUSE` (inert, no door), the pipe runs from it to the valve, and NAMES' rows
   show the name and its count only (C's rows already do).
2. **The alert's stamp is what the Pi did, mapped in Rust from the kind** (no new tracking):
   `threat` and `rebind` → `REFUSED` (the answer was blocked); `new_device` → `ADMITTED` (it joined
   and got the newcomer's default); `arp_spoof`, `dga`, `tunnel`, `beacon`, `burst` and any other kind
   → `OBSERVED` (nothing was blocked because of it). The precedence word is `URGENT` for High,
   `ROUTINE` otherwise. A threat's or rebind's slip: its name line is a door to NAME.
3. **Only readings the Pi has.** SYSTEM's figure under ROUTER is the band (`5 GHZ`), not the router's
   answer time; WI-FI's last line is `upstream answers in n ms` (§11.4). The spare slot's box says
   `KEPT` (the Pi does not know the spare's version); the box's line is `LAST UPDATE CONFIRMED` /
   `LAST UPDATE ROLLED BACK` / `NO UPDATE SINCE START`, with no date. On UPDATE the left box is the
   slot that served before the update and the right the new one, by their real letters; a version
   is written only for the build that runs. ALERTS' nothing-to-report lines are `LAST ENTRY FILED`
   (the store's last alert, left out when there never was one) and `WATCH OPENED` (this boot's
   start, the clock minus the uptime; left out while the clock is not set).
4. **Motion at rest is the log's cursor alone** (10×18, 4 frames on, 4 off at 8 fps; a write of
   18 rows). C's scope and sweep are not built (ALERTS is A's); UPDATE's DNS dot is drawn still;
   REJOIN's dashes follow the real link (all dark and `JOINING` until it is back), not a timer.
5. **The trial's figure** is `m:ss` to the trial's minimum time; past it, while the trial still
   waits, the figure names what it waits for (`WI-FI`, `CLOCK`, `UPSTREAM`).
6. **NIGHT with an alert waiting**: `NIGHT WATCH` becomes `n ON THE LOG` in white (A's night has no
   drop to whiten). An arriving alert still does not light the screen.
7. **A threat's NAME** has `BACK` on the left and `NEVER ALLOWED` (hollow, `G3`) where the right key
   would be (round 9's word replaces §3.7's `THREAT · STAYS`); the line is `On a threat list.
   Threats are never allowed.` The other NAME keys follow §3.7 (`KEEP REFUSED` + `ALLOW NAME`,
   `KEEP ANSWERED` + `REFUSE NAME`, and the take-back keys `REFUSE AGAIN` / `ANSWER AGAIN` on the
   owner's own entries); the padlock sits on the strong key and on the valve's handle.
8. **YOUR LIST's right figure** is the weekday the entry was set on the panel (`TODAY` for today);
   entries pushed from qnet have none. The Pi keeps that day beside the owner's list in qberrypi
   (memory only), never in qb-dns's answer-path structure.
9. **VERSION has no CHECK NOW** (§11.1), so A's VERSION page is not built; VERSION is the plain
   reading.

**Reference images:** the P2/P3 goldens (§9) are compared with the round 9 still of the concept
that draws them (`os-alerts` with `r9-A-alerts@2x.png`, `os-names` with `r9-C-names@2x.png`, `os-wifi`
with `r9-C-detail@2x.png`, …) and the plain readings with round 8's. `os-name-threat` is added.
The regions that differ by points 1, 3, 5 and 8 are listed as expected differences.
