//! The state that the update machinery shares: which slot runs, how far a trial has got, how the
//! last one ended, who may restart the board, and how to reach the card.
//!
//! One [`UpdateCtl`] is made in init's first step, from the kernel command line, and handed on
//! to everything that has a part in an update: the boot rules (init's step 3) fill it, the
//! statistics read it, the trial confirms or gives up through it, and the update receiver claims
//! it and asks for the restart through it. It holds no logic of the rules or of the card layout:
//! those are [`crate::slots`]'s.
//!
//! **Locks.** The small state sits behind one `std` mutex that is taken for a few assignments or
//! to copy a few values out, never across an `.await` and never while the card or the network is
//! used, and it is not the resolver's. The runtime's thread takes it only to answer a request of
//! `/api/v1/stats`.
//!
//! **Restarts.** The board restarts when the supervisor ends, and a [`Restart`] is how a
//! component makes it end: [`UpdateCtl::request_restart`] puts it in a channel that the
//! supervisor waits on ([`UpdateCtl::restart_receiver`]), and init restarts the board in the way
//! it names. The first request stands; a later one changes nothing.
//!
//! **The trial.** A boot that is on trial runs [`trial`]'s component, which confirms the build when
//! it has proved itself or gives up and asks for the restart that boots the slot it came from.
//!
//! **The card.** [`BootCard`] is init's use of the boot partition, lent to the other components
//! (the trial's confirm, the receiver's writes): each use is one claim of the card, one mount,
//! the work and one unmount, as init's own saves are, so no two uses of the card overlap. A use
//! blocks its thread, so the runtime's thread never makes one: it goes on the blocking pool.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;

use crate::config::TRIAL_CONFIRM_DEFAULT_SECS;
use crate::slots::{Marker, Slot};

pub mod receive;
pub mod trial;

/// Where the initramfs holds the digest of the build's boot files, as the build packs it: 64
/// lower-case hex digits and a line end (`slots::boot_files_digest`).
pub const BOOT_FILES_PATH: &str = "/etc/qberrypi/boot-files";

/// How a restart that the board is asked for goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Restart {
    /// An update was staged for the firmware's trial: restart with `0 tryboot`, so that the
    /// firmware reads `tryboot.txt` for one start (`sys::reboot_tryboot`).
    Tryboot,
    /// Restart as any restart goes: the firmware reads `config.txt`.
    Plain(Cause),
}

/// Why a [`Restart::Plain`] is asked for, which decides how init logs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cause {
    /// An update was staged for the software trial, which `config.txt` itself selects: logged at
    /// INFO, as the plan of the board.
    Update,
    /// The trial build gave up before it confirmed, for the reason given (one line, made by the
    /// trial itself and never a value from the configuration): logged at ERROR.
    GaveUp(String),
    /// The owner asked for it from the front panel (`RESTART` on UP): logged at INFO, as the owner's own
    /// decision. It is asked for only while no update is received and no trial is on
    /// ([`UpdateCtl::request_owner_restart`]).
    Owner,
}

impl std::fmt::Display for Restart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tryboot => f.write_str("an update was staged and is tried with tryboot"),
            Self::Plain(Cause::Update) => f.write_str("an update was staged and is tried"),
            Self::Plain(Cause::GaveUp(why)) => write!(f, "the trial gave up: {why}"),
            Self::Plain(Cause::Owner) => {
                f.write_str("the owner restarted the board from the panel")
            }
        }
    }
}

/// What a trial still waits for before it can confirm, as `/api/v1/stats` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// The Wi-Fi is not associated.
    Wifi,
    /// The clock is not set.
    Clock,
    /// No upstream has answered a forwarded question.
    Upstream,
    /// The minimum time since init started has not passed.
    Time,
}

impl Waiting {
    /// Every thing a trial can wait for, in the order the statistics list them.
    pub const ALL: [Waiting; 4] = [
        Waiting::Wifi,
        Waiting::Clock,
        Waiting::Upstream,
        Waiting::Time,
    ];

    /// The word the statistics use: `"wifi"`, `"clock"`, `"upstream"` or `"time"`.
    pub fn word(self) -> &'static str {
        match self {
            Waiting::Wifi => "wifi",
            Waiting::Clock => "clock",
            Waiting::Upstream => "upstream",
            Waiting::Time => "time",
        }
    }
}

/// A trial that this boot is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    /// The marker, as init found it with `started` appended.
    pub marker: Marker,
    /// When init started, which the trial's minimum time and its limit count from.
    pub began: Instant,
    /// What the trial still waits for; the trial component keeps it current.
    pub waiting_for: Vec<Waiting>,
    /// How long after `began` the trial's minimum time is up: [`TRIAL_CONFIRM_DEFAULT_SECS`] until the
    /// trial component says what its settings make of it ([`UpdateCtl::set_trial_minimum`]).
    pub confirm_after: Duration,
}

/// How an update's trial ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeKind {
    /// The trial build confirmed itself and is the slot that boots from now on.
    Confirmed,
    /// The trial failed and the slot it came from runs again.
    RolledBack,
}

impl OutcomeKind {
    /// The word the statistics use: `"confirmed"` or `"rolled_back"`.
    pub fn word(self) -> &'static str {
        match self {
            OutcomeKind::Confirmed => "confirmed",
            OutcomeKind::RolledBack => "rolled_back",
        }
    }
}

/// How the last trial that this boot knows of ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Which way it ended.
    pub kind: OutcomeKind,
    /// The build time of the trial build.
    pub build: u64,
    /// For a rollback, which case it was (`Marker::rollback_detail`); empty for a confirm.
    pub detail: String,
}

impl Outcome {
    /// A confirmed trial of `build`, with no detail.
    pub fn confirmed(build: u64) -> Self {
        Self {
            kind: OutcomeKind::Confirmed,
            build,
            detail: String::new(),
        }
    }

    /// A rollback away from the trial of `build`, for the reason `detail` says.
    pub fn rolled_back(build: u64, detail: impl Into<String>) -> Self {
        Self {
            kind: OutcomeKind::RolledBack,
            build,
            detail: detail.into(),
        }
    }
}

/// What the update machinery is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    /// Nothing: no update arrives and no trial is on.
    Idle,
    /// The receiver has the update, or has staged it and the board is about to restart.
    Receiving,
    /// This boot is the trial of an update.
    Trial(Trial),
}

/// What sits behind the lock.
struct State {
    phase: Phase,
    last: Option<Outcome>,
}

/// How far the receiver has got, for the panel's percentage: two numbers that the receiver writes and
/// the panel reads, with no lock between them. They are advisory: each stands alone, and a reader that
/// sees the one of a moment and the other of the next only draws a percentage a hair off.
#[derive(Default)]
struct Progress {
    /// The bytes of the body that have arrived, the manifest's and the files'.
    received: AtomicU64,
    /// The body's whole length, which the manifest's sizes add up to; 0 until the manifest is judged.
    length: AtomicU64,
}

impl Progress {
    fn reset(&self) {
        self.received.store(0, Ordering::Relaxed);
        self.length.store(0, Ordering::Relaxed);
    }
}

/// Why the receiver may not begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The card has no slots: the boot was of the old layout.
    NoSlots,
    /// An update is being received or has been staged, or a trial is on.
    Busy,
}

/// The use of the boot partition that init lends to the other components.
///
/// Nothing here knows how the card is mounted: init's implementation claims the card as its own
/// saves do (refused while a fatal error's rescue has it, waiting up to 5 s for another use),
/// mounts it, calls the work with the directory where it is, syncs after a writable use, and
/// unmounts. A test lends a directory instead.
pub trait BootCard: Send + Sync {
    /// Uses the boot partition once: mounts it as `access` says, runs `work` on the directory
    /// where its files are, and unmounts it.
    ///
    /// An error is the claim being refused or the mount failing, and `work` has not run; what the
    /// work finds or fails at is the work's own to keep. Blocks for as long as the use takes, so
    /// it must not be called on the runtime's thread.
    fn with_boot(&self, access: CardAccess, work: &mut dyn FnMut(&Path)) -> io::Result<()>;
}

/// What a use of the boot partition may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardAccess {
    /// The partition is mounted read-only and nothing is written.
    ReadOnly,
    /// The partition is mounted writable, and synced before it is unmounted.
    Writable,
}

/// The card of a board that has none to lend, as the old layout and a test that never touches
/// the card have: every use fails with `Unsupported`, and the work does not run.
pub struct NoCard;

impl BootCard for NoCard {
    fn with_boot(&self, _: CardAccess, _: &mut dyn FnMut(&Path)) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "this board has no card to lend",
        ))
    }
}

/// The update machinery's shared state: see the module's text.
pub struct UpdateCtl {
    slot: Option<Slot>,
    boot_files: Option<String>,
    card: Arc<dyn BootCard>,
    state: Mutex<State>,
    /// How much of the update the receiver has taken in; read without the lock.
    progress: Progress,
    restart: watch::Sender<Option<Restart>>,
}

impl UpdateCtl {
    /// The state of a boot on `slot` (`None` for the old layout), idle, with no outcome yet.
    ///
    /// `boot_files` is the digest the build packed (`BOOT_FILES_PATH`), `None` if the boot has no
    /// slot or the file was missing or unusable. `card` is what the components other than init's
    /// own steps use the boot partition by.
    pub fn new(
        slot: Option<Slot>,
        boot_files: Option<String>,
        card: Arc<dyn BootCard>,
    ) -> Arc<Self> {
        Arc::new(Self {
            slot,
            boot_files,
            card,
            state: Mutex::new(State {
                phase: Phase::Idle,
                last: None,
            }),
            progress: Progress::default(),
            restart: watch::channel(None).0,
        })
    }

    /// The state of the old layout, which has no slots, no digest and no card to lend.
    pub fn off() -> Arc<Self> {
        Self::new(None, None, Arc::new(NoCard))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The slot this boot runs, from the kernel command line; `None` on the old layout.
    pub fn slot(&self) -> Option<Slot> {
        self.slot
    }

    /// The digest of the build's boot files, 64 lower-case hex digits, if the boot has one.
    pub fn boot_files(&self) -> Option<&str> {
        self.boot_files.as_deref()
    }

    /// The boot partition, for a component's use of it (see [`BootCard`]).
    pub fn card(&self) -> &dyn BootCard {
        self.card.as_ref()
    }

    /// Records that this boot is on trial, from `began`, which is when init started: nothing is
    /// known yet, so the trial waits for every one of [`Waiting::ALL`].
    pub fn begin_trial(&self, marker: Marker, began: Instant) {
        self.state().phase = Phase::Trial(Trial {
            marker,
            began,
            waiting_for: Waiting::ALL.to_vec(),
            confirm_after: Duration::from_secs(TRIAL_CONFIRM_DEFAULT_SECS),
        });
    }

    /// Says how long after init started the trial's minimum time is up, for the time that
    /// [`View`] shows left (spec §12.5); nothing happens if this boot is not on trial. The trial
    /// component calls it with the same figure it confirms by.
    pub fn set_trial_minimum(&self, confirm_after: Duration) {
        if let Phase::Trial(trial) = &mut self.state().phase {
            trial.confirm_after = confirm_after;
        }
    }

    /// A copy of the trial this boot is on, if it is on one.
    pub fn trial(&self) -> Option<Trial> {
        match &self.state().phase {
            Phase::Trial(trial) => Some(trial.clone()),
            _ => None,
        }
    }

    /// Replaces what the trial still waits for; nothing happens if this boot is not on trial.
    pub fn set_waiting(&self, waiting_for: Vec<Waiting>) {
        if let Phase::Trial(trial) = &mut self.state().phase {
            trial.waiting_for = waiting_for;
        }
    }

    /// Records how a trial ended: the last outcome is `outcome`, and a trial that this boot was
    /// on is over, so the machinery is idle. Another phase (an update being received) is left.
    pub fn finish(&self, outcome: Outcome) {
        let mut state = self.state();
        if matches!(state.phase, Phase::Trial(_)) {
            state.phase = Phase::Idle;
        }
        state.last = Some(outcome);
    }

    /// Takes the machinery for the receiver, which holds it until the returned guard is dropped.
    ///
    /// Refused when the boot has no slots, and when an update is already being received or
    /// staged or a trial is on; nothing changes then. The check and the taking are one step.
    pub fn begin_receive(self: &Arc<Self>) -> Result<Receiving, Refused> {
        if self.slot.is_none() {
            return Err(Refused::NoSlots);
        }
        let mut state = self.state();
        if state.phase != Phase::Idle {
            return Err(Refused::Busy);
        }
        state.phase = Phase::Receiving;
        self.progress.reset();
        Ok(Receiving {
            ctl: self.clone(),
            staged: false,
        })
    }

    /// Asks the supervisor to end so that the board restarts as `restart` says; says whether
    /// this request is the one that stands. A request made after another changes nothing.
    pub fn request_restart(&self, restart: Restart) -> bool {
        self.restart.send_if_modified(|asked| {
            if asked.is_some() {
                return false;
            }
            *asked = Some(restart);
            true
        })
    }

    /// Asks for the restart that the owner chose on the front panel (`Cause::Owner`): refused with
    /// `Err(Busy)` while an update is received or staged and while this boot is on trial, which a restart
    /// would cut short. The look at the state and the request are one step, so that an update that begins
    /// at this moment is the one refused. A restart that stands already is asked for again to no effect,
    /// and that is `Ok`: the board is restarting, as the owner wished.
    pub fn request_owner_restart(&self) -> Result<(), Busy> {
        let state = self.state();
        if state.phase != Phase::Idle {
            return Err(Busy);
        }
        // The first request stands: a later one changes nothing, as this one may find.
        self.request_restart(Restart::Plain(Cause::Owner));
        Ok(())
    }

    /// Runs `act` if no update is received or staged and this boot is not on a trial, and `Err(Busy)` if
    /// one is. The look at the state and `act` are one step, as in [`UpdateCtl::request_owner_restart`], so
    /// that an update that begins at this moment is the one refused. `act` runs with the state held: it must
    /// be short, and must not ask this control for anything.
    pub fn when_idle<T>(&self, act: impl FnOnce() -> T) -> Result<T, Busy> {
        let state = self.state();
        if state.phase != Phase::Idle {
            return Err(Busy);
        }
        Ok(act())
    }

    /// The end of the channel that the supervisor waits on (`supervisor::Plan::restart`).
    pub fn restart_receiver(&self) -> watch::Receiver<Option<Restart>> {
        self.restart.subscribe()
    }

    /// Copies out what the statistics tell and the panel's UPDATE screens draw, under the lock and no
    /// longer than that takes (the receiver's progress is two atomics, read after it).
    pub fn view(&self) -> View {
        let (state, trial, last) = {
            let state = self.state();
            let (word, trial) = match &state.phase {
                Phase::Idle => ("idle", None),
                Phase::Receiving => ("receiving", None),
                Phase::Trial(trial) => ("trial", Some(trial.clone())),
            };
            (word, trial, state.last.clone())
        };
        let (received, length) = if state == "receiving" {
            (
                self.progress.received.load(Ordering::Relaxed),
                self.progress.length.load(Ordering::Relaxed),
            )
        } else {
            (0, 0)
        };
        View {
            slot: self.slot,
            boot_files: self.boot_files.clone(),
            state,
            received,
            length,
            trial: trial.map(|trial| {
                let elapsed = trial.began.elapsed();
                TrialView {
                    build: trial.marker.build,
                    from: trial.marker.from,
                    secs: elapsed.as_secs(),
                    min_left_secs: whole_secs_up(trial.confirm_after.saturating_sub(elapsed)),
                    waiting_for: trial.waiting_for,
                }
            }),
            last,
        }
    }
}

/// The seconds of `left`, counted up: a trial that has 0.2 s to go has 1 s left on the screen, and
/// shows 0 only once its minimum time is up.
fn whole_secs_up(left: Duration) -> u64 {
    left.as_secs() + u64::from(left.subsec_nanos() > 0)
}

/// An update is being received or staged, or this boot is on trial: no restart is made now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy;

/// The machinery taken for the receiver; dropping it gives it back, unless the update was staged.
#[must_use = "the machinery is the receiver's for as long as this lives"]
pub struct Receiving {
    ctl: Arc<UpdateCtl>,
    staged: bool,
}

impl Receiving {
    /// Says that the update was staged and the board is about to restart: the machinery stays
    /// taken, so that no second update is received in the time that is left.
    pub fn staged(mut self) {
        self.staged = true;
    }

    /// Counts `bytes` more of the update as arrived, for the percentage the panel shows.
    pub fn arrived(&self, bytes: usize) {
        self.ctl
            .progress
            .received
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Says how long the whole update is, in bytes: the manifest's sizes added up, once the manifest is
    /// judged.
    pub fn length_is(&self, length: u64) {
        self.ctl.progress.length.store(length, Ordering::Relaxed);
    }
}

impl Drop for Receiving {
    fn drop(&mut self) {
        if self.staged {
            return;
        }
        let mut state = self.ctl.state();
        if state.phase == Phase::Receiving {
            state.phase = Phase::Idle;
            self.ctl.progress.reset();
        }
    }
}

/// What `/api/v1/stats` and the front panel's UPDATE screens tell of the update machinery, copied out of
/// [`UpdateCtl::view`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// The running slot; `None` on the old layout.
    pub slot: Option<Slot>,
    /// The digest of the build's boot files, if there is one.
    pub boot_files: Option<String>,
    /// `"idle"`, `"receiving"` or `"trial"`.
    pub state: &'static str,
    /// While an update is received (or staged): the bytes of it that have arrived; 0 otherwise.
    pub received: u64,
    /// While an update is received (or staged): its whole length in bytes, from the manifest; 0 until the
    /// manifest is judged, and otherwise.
    pub length: u64,
    /// The trial this boot is on, if it is on one.
    pub trial: Option<TrialView>,
    /// How the last trial ended, if this boot knows of one.
    pub last: Option<Outcome>,
}

/// The trial as the statistics show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialView {
    /// The build time of the trial build.
    pub build: u64,
    /// The slot a rollback boots.
    pub from: Slot,
    /// How long ago init started, in whole seconds.
    pub secs: u64,
    /// The seconds until the trial's minimum time is up, counted up; 0 once it is (what the trial still
    /// waits for is then `waiting_for`).
    pub min_left_secs: u64,
    /// What the trial still waits for.
    pub waiting_for: Vec<Waiting>,
}

/// Reads the digest that the build packed, from `path`: the file's text if it is 64 lower-case
/// hex digits and a line end (or none), else `None`.
///
/// No more than 66 bytes are read, so a stray file is not read whole; a file that is missing,
/// is not a file, or holds anything else gives `None`.
pub fn read_boot_files(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(66).read_to_string(&mut text).ok()?;
    let digits = text.strip_suffix('\n').unwrap_or(&text);
    let valid = digits.len() == 64
        && digits
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    valid.then(|| digits.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::slots::Mode;

    fn marker(slot: Slot) -> Marker {
        Marker {
            slot,
            from: slot.other(),
            build: 1_794_470_000,
            mode: Mode::Tryboot,
            started: true,
            failed: None,
        }
    }

    fn on_slot(slot: Slot) -> Arc<UpdateCtl> {
        UpdateCtl::new(Some(slot), Some("ab".repeat(32)), Arc::new(NoCard))
    }

    #[test]
    fn a_new_state_is_idle_and_knows_its_slot_and_digest() {
        let ctl = on_slot(Slot::A);
        assert_eq!(ctl.slot(), Some(Slot::A));
        assert_eq!(ctl.boot_files(), Some("ab".repeat(32).as_str()));
        assert_eq!(
            ctl.view(),
            View {
                slot: Some(Slot::A),
                boot_files: Some("ab".repeat(32)),
                state: "idle",
                received: 0,
                length: 0,
                trial: None,
                last: None,
            }
        );
    }

    #[test]
    fn the_old_layout_has_no_slot_no_digest_and_no_card_to_lend() {
        let ctl = UpdateCtl::off();
        assert_eq!((ctl.slot(), ctl.boot_files()), (None, None));
        let view = ctl.view();
        assert_eq!(
            (view.slot, view.boot_files, view.state),
            (None, None, "idle")
        );
        let mut ran = false;
        let error = ctl
            .card()
            .with_boot(CardAccess::Writable, &mut |_| ran = true)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(!ran);
    }

    #[test]
    fn a_trial_is_seen_with_its_build_its_origin_and_all_it_waits_for() {
        let ctl = on_slot(Slot::B);
        ctl.begin_trial(marker(Slot::B), Instant::now());
        let view = ctl.view();
        assert_eq!(view.state, "trial");
        let trial = view.trial.unwrap();
        assert_eq!(
            (trial.build, trial.from, trial.waiting_for),
            (1_794_470_000, Slot::A, Waiting::ALL.to_vec())
        );
        assert!(trial.secs < 5);
        assert_eq!(ctl.trial().unwrap().marker, marker(Slot::B));
    }

    #[tokio::test(start_paused = true)]
    async fn the_seconds_of_a_trial_count_from_when_init_started() {
        let ctl = on_slot(Slot::B);
        let began = Instant::now();
        ctl.begin_trial(marker(Slot::B), began);
        tokio::time::advance(Duration::from_secs(95)).await;
        assert_eq!(ctl.view().trial.unwrap().secs, 95);
    }

    #[test]
    fn what_a_trial_waits_for_is_replaced_and_ignored_when_there_is_no_trial() {
        let ctl = on_slot(Slot::B);
        ctl.set_waiting(vec![Waiting::Wifi]);
        assert_eq!(ctl.view().trial, None);
        ctl.begin_trial(marker(Slot::B), Instant::now());
        ctl.set_waiting(vec![Waiting::Clock, Waiting::Time]);
        assert_eq!(
            ctl.view().trial.unwrap().waiting_for,
            [Waiting::Clock, Waiting::Time]
        );
        ctl.set_waiting(Vec::new());
        assert_eq!(ctl.view().trial.unwrap().waiting_for, []);
    }

    #[test]
    fn the_words_the_statistics_use_are_the_designs() {
        let words: Vec<&str> = Waiting::ALL.iter().map(|w| w.word()).collect();
        assert_eq!(words, ["wifi", "clock", "upstream", "time"]);
        assert_eq!(OutcomeKind::Confirmed.word(), "confirmed");
        assert_eq!(OutcomeKind::RolledBack.word(), "rolled_back");
    }

    #[test]
    fn finishing_a_trial_records_the_outcome_and_goes_idle() {
        let ctl = on_slot(Slot::B);
        ctl.begin_trial(marker(Slot::B), Instant::now());
        ctl.finish(Outcome::confirmed(7));
        let view = ctl.view();
        assert_eq!((view.state, view.trial), ("idle", None));
        assert_eq!(view.last, Some(Outcome::confirmed(7)));
        // A rollback replaces what was there; it needs no trial to be on.
        ctl.finish(Outcome::rolled_back(8, "the trial build never started"));
        assert_eq!(
            ctl.view().last,
            Some(Outcome::rolled_back(8, "the trial build never started"))
        );
        assert_eq!(ctl.view().state, "idle");
    }

    #[test]
    fn finishing_a_trial_does_not_end_a_receive() {
        let ctl = on_slot(Slot::A);
        let receiving = ctl.begin_receive().unwrap();
        ctl.finish(Outcome::confirmed(1));
        assert_eq!(ctl.view().state, "receiving");
        drop(receiving);
    }

    #[test]
    fn the_receiver_takes_the_machinery_and_gives_it_back() {
        let ctl = on_slot(Slot::A);
        let receiving = ctl.begin_receive().unwrap();
        assert_eq!(ctl.view().state, "receiving");
        assert_eq!(ctl.begin_receive().err(), Some(Refused::Busy));
        drop(receiving);
        assert_eq!(ctl.view().state, "idle");
        assert!(ctl.begin_receive().is_ok());
    }

    #[test]
    fn a_staged_update_keeps_the_machinery_taken() {
        let ctl = on_slot(Slot::A);
        ctl.begin_receive().unwrap().staged();
        assert_eq!(ctl.view().state, "receiving");
        assert_eq!(ctl.begin_receive().err(), Some(Refused::Busy));
    }

    #[test]
    fn no_update_is_received_on_a_trial_or_on_a_card_without_slots() {
        let ctl = on_slot(Slot::B);
        ctl.begin_trial(marker(Slot::B), Instant::now());
        assert_eq!(ctl.begin_receive().err(), Some(Refused::Busy));
        // The refusal leaves the trial as it was.
        assert_eq!(ctl.view().state, "trial");
        assert_eq!(
            UpdateCtl::off().begin_receive().err(),
            Some(Refused::NoSlots)
        );
    }

    #[test]
    fn the_first_request_to_restart_stands() {
        let ctl = on_slot(Slot::A);
        let mut asked = ctl.restart_receiver();
        assert_eq!(*asked.borrow(), None);
        assert!(ctl.request_restart(Restart::Tryboot));
        assert!(!ctl.request_restart(Restart::Plain(Cause::GaveUp("late".into()))));
        assert!(asked.has_changed().unwrap());
        assert_eq!(*asked.borrow_and_update(), Some(Restart::Tryboot));
        // A receiver made later sees what stands.
        assert_eq!(*ctl.restart_receiver().borrow(), Some(Restart::Tryboot));
    }

    #[test]
    fn a_restart_is_described_in_words_that_name_the_cause() {
        assert!(Restart::Tryboot.to_string().contains("tryboot"));
        assert!(
            !Restart::Plain(Cause::Update)
                .to_string()
                .contains("tryboot")
        );
        assert_eq!(
            Restart::Plain(Cause::GaveUp("no Wi-Fi".into())).to_string(),
            "the trial gave up: no Wi-Fi"
        );
        assert_eq!(
            Restart::Plain(Cause::Owner).to_string(),
            "the owner restarted the board from the panel"
        );
    }

    #[test]
    fn the_owners_restart_is_asked_for_as_a_plain_restart_with_its_own_cause() {
        let ctl = on_slot(Slot::A);
        let mut asked = ctl.restart_receiver();
        assert_eq!(ctl.request_owner_restart(), Ok(()));
        assert_eq!(
            *asked.borrow_and_update(),
            Some(Restart::Plain(Cause::Owner))
        );
        // A second one is no refusal: the board is restarting, as asked. The first stands.
        assert_eq!(ctl.request_owner_restart(), Ok(()));
        assert!(!asked.has_changed().unwrap());
    }

    #[test]
    fn the_owners_restart_is_refused_while_an_update_is_received_or_staged_or_on_a_trial() {
        let nothing_asked = |ctl: &UpdateCtl| ctl.restart_receiver().borrow().is_none();
        // While the receiver has the machinery.
        let ctl = on_slot(Slot::A);
        let receiving = ctl.begin_receive().unwrap();
        assert_eq!(ctl.request_owner_restart(), Err(Busy));
        assert!(nothing_asked(&ctl));
        // After it is given back the restart is let through.
        drop(receiving);
        assert_eq!(ctl.request_owner_restart(), Ok(()));
        assert!(!nothing_asked(&ctl));

        // While a staged update waits for its restart.
        let ctl = on_slot(Slot::A);
        ctl.begin_receive().unwrap().staged();
        assert_eq!(ctl.request_owner_restart(), Err(Busy));
        assert!(nothing_asked(&ctl));

        // While this boot is on trial; once the trial has ended it is let through.
        let ctl = on_slot(Slot::B);
        ctl.begin_trial(marker(Slot::B), Instant::now());
        assert_eq!(ctl.request_owner_restart(), Err(Busy));
        assert!(nothing_asked(&ctl));
        ctl.finish(Outcome::confirmed(1));
        assert_eq!(ctl.request_owner_restart(), Ok(()));
    }

    #[test]
    fn an_act_is_done_only_while_no_update_is_received_or_staged_or_on_a_trial() {
        let ctl = on_slot(Slot::A);
        assert_eq!(ctl.when_idle(|| 7), Ok(7));
        let receiving = ctl.begin_receive().unwrap();
        assert_eq!(
            ctl.when_idle(|| unreachable!("an update is received")),
            Err(Busy)
        );
        drop(receiving);
        assert_eq!(ctl.when_idle(|| 8), Ok(8));
        ctl.begin_receive().unwrap().staged();
        assert_eq!(
            ctl.when_idle(|| unreachable!("an update is staged")),
            Err(Busy)
        );
        let ctl = on_slot(Slot::B);
        ctl.begin_trial(marker(Slot::B), Instant::now());
        assert_eq!(ctl.when_idle(|| unreachable!("a trial runs")), Err(Busy));
        ctl.finish(Outcome::confirmed(1));
        assert_eq!(ctl.when_idle(|| 9), Ok(9));
    }

    #[test]
    fn the_receivers_progress_is_counted_and_shown_only_while_it_has_the_machinery() {
        let ctl = on_slot(Slot::A);
        let idle = ctl.view();
        assert_eq!((idle.received, idle.length), (0, 0));
        let receiving = ctl.begin_receive().unwrap();
        // Nothing has arrived, and the length is not known until the manifest is judged.
        assert_eq!((ctl.view().received, ctl.view().length), (0, 0));
        receiving.arrived(1_000);
        receiving.arrived(24);
        let view = ctl.view();
        assert_eq!(
            (view.state, view.received, view.length),
            ("receiving", 1_024, 0)
        );
        receiving.length_is(4_096);
        receiving.arrived(1_024);
        let view = ctl.view();
        assert_eq!((view.received, view.length), (2_048, 4_096));
        // An update that is refused or fails gives the machinery back and its progress with it.
        drop(receiving);
        let view = ctl.view();
        assert_eq!((view.state, view.received, view.length), ("idle", 0, 0));
        // The next one starts from nothing.
        let receiving = ctl.begin_receive().unwrap();
        assert_eq!((ctl.view().received, ctl.view().length), (0, 0));
        // One that is staged keeps the whole of what it counted, for the screen that waits for the restart.
        receiving.length_is(10);
        receiving.arrived(10);
        receiving.staged();
        let view = ctl.view();
        assert_eq!(
            (view.state, view.received, view.length),
            ("receiving", 10, 10)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_trial_shows_the_time_left_to_its_minimum_and_then_none() {
        let ctl = on_slot(Slot::B);
        ctl.begin_trial(marker(Slot::B), Instant::now());
        // Until the trial component says what its settings make of it, the minimum is the default 120 s.
        assert_eq!(ctl.view().trial.unwrap().min_left_secs, 120);
        tokio::time::advance(Duration::from_secs(95)).await;
        assert_eq!(ctl.view().trial.unwrap().min_left_secs, 25);
        // The trial component's own figure, counted from the same start.
        ctl.set_trial_minimum(Duration::from_secs(30));
        assert_eq!(ctl.view().trial.unwrap().min_left_secs, 0);
        ctl.set_trial_minimum(Duration::from_secs(100));
        assert_eq!(ctl.view().trial.unwrap().min_left_secs, 5);
        // Counted up: with half a second to go there is still 1 s on the screen, and 0 only once it is time.
        tokio::time::advance(Duration::from_millis(4_500)).await;
        assert_eq!(ctl.view().trial.unwrap().min_left_secs, 1);
        tokio::time::advance(Duration::from_millis(500)).await;
        assert_eq!(ctl.view().trial.unwrap().min_left_secs, 0);
        // What it waits for past the minimum is the view's `waiting_for`, as before.
        ctl.set_waiting(vec![Waiting::Upstream]);
        let trial = ctl.view().trial.unwrap();
        assert_eq!(
            (trial.min_left_secs, trial.waiting_for),
            (0, vec![Waiting::Upstream])
        );
    }

    #[test]
    fn the_trial_minimum_is_set_only_on_a_trial() {
        let ctl = on_slot(Slot::A);
        ctl.set_trial_minimum(Duration::from_secs(5));
        assert_eq!(ctl.view().trial, None);
        assert_eq!(ctl.view().state, "idle");
    }

    #[test]
    fn a_lent_card_is_used_through_the_trait() {
        struct Counting(AtomicUsize);
        impl BootCard for Counting {
            fn with_boot(&self, access: CardAccess, work: &mut dyn FnMut(&Path)) -> io::Result<()> {
                self.0.fetch_add(1, Ordering::SeqCst);
                if access == CardAccess::Writable {
                    work(Path::new("/nowhere"));
                }
                Ok(())
            }
        }
        let card = Arc::new(Counting(AtomicUsize::new(0)));
        let ctl = UpdateCtl::new(Some(Slot::A), None, card.clone());
        let mut seen = None;
        ctl.card()
            .with_boot(CardAccess::Writable, &mut |dir| seen = Some(dir.to_owned()))
            .unwrap();
        assert_eq!(seen.as_deref(), Some(Path::new("/nowhere")));
        ctl.card()
            .with_boot(CardAccess::ReadOnly, &mut |_| {})
            .unwrap();
        assert_eq!(card.0.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_packed_digest_is_read_only_when_it_is_one() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("boot-files");
        let digest = "0123456789abcdef".repeat(4);
        std::fs::write(&path, format!("{digest}\n")).unwrap();
        assert_eq!(read_boot_files(&path), Some(digest.clone()));
        std::fs::write(&path, &digest).unwrap();
        assert_eq!(read_boot_files(&path), Some(digest.clone()));
        for bad in [
            "".to_owned(),
            "0123".to_owned(),
            digest.to_uppercase(),
            format!("{digest}0"),
            format!("{digest}\n\n"),
            format!("{}g", &digest[..63]),
            format!(" {digest}"),
        ] {
            std::fs::write(&path, &bad).unwrap();
            assert_eq!(read_boot_files(&path), None, "{bad:?}");
        }
        assert_eq!(read_boot_files(&tmp.path().join("absent")), None);
        // A file that is not a file is no digest either.
        assert_eq!(read_boot_files(tmp.path()), None);
    }
}
