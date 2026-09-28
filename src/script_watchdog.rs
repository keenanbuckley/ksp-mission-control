//! Liveness tracking for scripts dispatched to kOS.
//!
//! kerboscript has no try/catch, and a CPU that loses power runs no cleanup,
//! so a script that dies mid-flight never sends `script_done`. The dispatcher
//! instead heartbeats about once per real-time second while a script runs,
//! and answers server pings while idle. This module turns those heartbeats
//! plus the UT stream into synthetic failures and a link state.
//!
//! Silence is measured in wall time, but only accumulates across UT samples
//! that advanced: a paused game freezes UT and kOS together, so a pause must
//! not count against the deadline. A UT rewind means a save was loaded, which
//! kills every running script at once.
//!
//! Liveness is tracked per CPU session (`boot`). A script dies only when its
//! own CPU goes quiet, so a second dispatcher on another vessel can't keep it
//! alive, and a vessel switch doesn't kill a script that is still running.

use std::time::{Duration, Instant};

use serde::Serialize;

/// Wall time, counted only while UT advances, after which a silent CPU session
/// is declared dead. The dispatcher beats at 1 Hz.
pub const HEARTBEAT_DEADLINE: Duration = Duration::from_secs(10);

/// Cap on the wall time one UT sample can add to a silence counter. The UT
/// stream runs at a few Hz, so a larger gap means the game was paused or kRPC
/// reconnected, and that gap must not count as silence.
const MAX_UT_SAMPLE_GAP: Duration = Duration::from_secs(1);

pub const REASON_SILENT: &str = "no heartbeat from kOS";
pub const REASON_UT_REWIND: &str = "UT went backwards (save loaded)";

/// Whether the active vessel's dispatcher is answering, and what it is running.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct KosLink {
    pub up: bool,
    pub running: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Failure {
    pub path: String,
    pub reason: &'static str,
}

/// One heartbeat from a dispatcher. `path` is `None` while idle. `active` is
/// whether the sending CPU is on the active vessel, the only one commands and
/// pings reach.
#[derive(Clone, Copy, Debug)]
pub struct Heartbeat<'a> {
    pub boot: &'a str,
    pub path: Option<&'a str>,
    pub active: bool,
}

#[derive(Debug)]
struct Entry {
    path: String,
    boot: Option<String>,
    silence: Duration,
}

#[derive(Debug, Default)]
pub struct ScriptWatchdog {
    entries: Vec<Entry>,
    link: KosLink,
    link_boot: Option<String>,
    link_silence: Duration,
    last_ut: Option<(f64, Instant)>,
    ut_at_last_ping: Option<f64>,
}

impl ScriptWatchdog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn link(&self) -> &KosLink {
        &self.link
    }

    /// A `run_script` was sent to the active vessel. The entry belongs to the
    /// CPU session currently answering, or to the next one that answers if the
    /// link is down.
    pub fn on_dispatch(&mut self, path: String) {
        let boot = if self.link.up {
            self.link_boot.clone()
        } else {
            None
        };
        self.entries.push(Entry {
            path,
            boot,
            silence: Duration::ZERO,
        });
    }

    /// A real `script_done` arrived. With a boot, it matches an entry of that
    /// session, or an entry no session has claimed yet (a script rejected
    /// before its first heartbeat). An event whose boot matches neither is
    /// stale: kIPC persists its queue into the save file and replays it after
    /// a load, so it must not remove a live run of the same script. Without a
    /// boot, it matches the oldest entry with that path.
    pub fn on_done(&mut self, path: &str, boot: Option<&str>) {
        let found = match boot {
            Some(b) => self
                .entries
                .iter()
                .position(|e| e.path == path && e.boot.as_deref() == Some(b))
                .or_else(|| {
                    self.entries
                        .iter()
                        .position(|e| e.path == path && e.boot.is_none())
                }),
            None => self.entries.iter().position(|e| e.path == path),
        };
        if let Some(i) = found {
            self.entries.remove(i);
        }
        // A clean finish sends no idle heartbeat, so this is the only signal
        // that the link is idle again and pings can resume.
        let same_session = boot.is_none() || boot == self.link_boot.as_deref();
        if same_session && self.link.running.as_deref() == Some(path) {
            self.link.running = None;
        }
    }

    pub fn on_heartbeat(&mut self, hb: Heartbeat) {
        for e in &mut self.entries {
            match &e.boot {
                Some(b) if b == hb.boot => e.silence = Duration::ZERO,
                None if hb.active => {
                    e.boot = Some(hb.boot.to_owned());
                    e.silence = Duration::ZERO;
                }
                _ => {}
            }
        }
        if hb.active {
            self.link = KosLink {
                up: true,
                running: hb.path.map(str::to_owned),
            };
            self.link_boot = Some(hb.boot.to_owned());
            self.link_silence = Duration::ZERO;
        }
    }

    /// Feed one UT sample. Returns the scripts declared dead by it.
    pub fn on_ut(&mut self, ut: f64, now: Instant) -> Vec<Failure> {
        let Some((prev_ut, prev_at)) = self.last_ut.replace((ut, now)) else {
            return Vec::new();
        };
        if ut < prev_ut {
            self.link_down();
            return self
                .entries
                .drain(..)
                .map(|e| Failure {
                    path: e.path,
                    reason: REASON_UT_REWIND,
                })
                .collect();
        }
        if ut == prev_ut {
            return Vec::new();
        }

        let dt = now
            .saturating_duration_since(prev_at)
            .min(MAX_UT_SAMPLE_GAP);
        self.link_silence += dt;
        if self.link.up && self.link_silence > HEARTBEAT_DEADLINE {
            self.link_down();
        }
        for e in &mut self.entries {
            e.silence += dt;
        }
        let (dead, alive): (Vec<_>, Vec<_>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|e| e.silence > HEARTBEAT_DEADLINE);
        self.entries = alive;
        dead.into_iter()
            .map(|e| Failure {
                path: e.path,
                reason: REASON_SILENT,
            })
            .collect()
    }

    /// Whether to ping the active vessel now. Never while a script runs (the
    /// dispatcher can't answer until it returns) and never twice for the same
    /// UT (a paused game would queue pings for a burst of replies on resume).
    pub fn ping_due(&mut self) -> bool {
        if self.link.running.is_some() {
            return false;
        }
        let Some((ut, _)) = self.last_ut else {
            return false;
        };
        if self.ut_at_last_ping == Some(ut) {
            return false;
        }
        self.ut_at_last_ping = Some(ut);
        true
    }

    fn link_down(&mut self) {
        self.link = KosLink::default();
        self.link_boot = None;
        self.link_silence = Duration::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(200);

    /// Drives the watchdog with a UT stream at 5 Hz wall time.
    struct Clock {
        t0: Instant,
        wall: Duration,
        ut: f64,
    }

    impl Clock {
        fn new(wd: &mut ScriptWatchdog) -> Self {
            let mut c = Self {
                t0: Instant::now(),
                wall: Duration::ZERO,
                ut: 1000.0,
            };
            assert!(wd.on_ut(c.ut, c.t0).is_empty());
            c.wall += TICK;
            c
        }

        fn now(&self) -> Instant {
            self.t0 + self.wall
        }

        /// Advance wall time by `wall`, one UT sample per tick, UT moving by
        /// `ut_per_tick` each sample. Returns all failures.
        fn run(
            &mut self,
            wd: &mut ScriptWatchdog,
            wall: Duration,
            ut_per_tick: f64,
        ) -> Vec<Failure> {
            let mut out = Vec::new();
            let end = self.wall + wall;
            while self.wall <= end {
                self.ut += ut_per_tick;
                out.extend(wd.on_ut(self.ut, self.now()));
                self.wall += TICK;
            }
            out
        }
    }

    fn beat<'a>(boot: &'a str, path: Option<&'a str>) -> Heartbeat<'a> {
        Heartbeat {
            boot,
            path,
            active: true,
        }
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn silence_while_ut_advances_fails_after_deadline() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_dispatch("launch.ks".into());
        assert!(c.run(&mut wd, secs(9), 0.2).is_empty());
        let failed = c.run(&mut wd, secs(2), 0.2);
        assert_eq!(
            failed,
            vec![Failure {
                path: "launch.ks".into(),
                reason: REASON_SILENT
            }]
        );
    }

    #[test]
    fn pause_freezes_the_deadline() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_dispatch("launch.ks".into());
        assert!(wd.ping_due());
        // Ten minutes of wall time with UT frozen.
        assert!(c.run(&mut wd, secs(600), 0.0).is_empty());
        assert!(!wd.ping_due());
        assert!(c.run(&mut wd, secs(5), 0.2).is_empty());
    }

    #[test]
    fn resume_gap_adds_at_most_one_second() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_dispatch("launch.ks".into());
        assert!(c.run(&mut wd, secs(8), 0.2).is_empty());
        // No samples at all for five minutes (paused, stream quiet), then one.
        c.wall += secs(300);
        c.ut += 0.2;
        assert!(wd.on_ut(c.ut, c.now()).is_empty());
        c.wall += TICK;
        // 8 s + at most 1 s so far; a further second still fits under 10 s.
        assert!(c.run(&mut wd, Duration::from_millis(400), 0.2).is_empty());
    }

    #[test]
    fn rails_warp_with_steady_beats_stays_alive() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_heartbeat(beat("a", None));
        wd.on_dispatch("maneuver.ks".into());
        for _ in 0..120 {
            // 100000x warp: 20000 s of UT per 0.2 s sample.
            assert!(c.run(&mut wd, secs(1), 20_000.0).is_empty());
            wd.on_heartbeat(beat("a", Some("maneuver.ks")));
        }
        assert!(wd.link().up);
    }

    #[test]
    fn ut_rewind_fails_everything_immediately() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_heartbeat(beat("a", None));
        wd.on_dispatch("launch.ks".into());
        wd.on_dispatch("maneuver.ks".into());
        c.run(&mut wd, secs(1), 0.2);
        let failed = wd.on_ut(c.ut - 500.0, c.now());
        assert_eq!(failed.len(), 2);
        assert!(failed.iter().all(|f| f.reason == REASON_UT_REWIND));
        assert!(!wd.link().up);
    }

    #[test]
    fn done_matches_path_and_boot_then_path() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_heartbeat(beat("a", None));
        wd.on_dispatch("launch.ks".into());
        wd.on_heartbeat(beat("b", None));
        wd.on_dispatch("launch.ks".into());
        // b's run finishes first; a's must remain.
        wd.on_done("launch.ks", Some("b"));
        let failed = c.run(&mut wd, secs(11), 0.2);
        assert_eq!(failed.len(), 1);

        wd.on_heartbeat(beat("c", None));
        wd.on_dispatch("maneuver.ks".into());
        wd.on_done("maneuver.ks", None);
        assert!(c.run(&mut wd, secs(20), 0.2).is_empty());
    }

    #[test]
    fn stale_done_from_a_dead_session_leaves_live_run_watched() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_heartbeat(beat("new", None));
        wd.on_dispatch("launch.ks".into());
        wd.on_heartbeat(beat("new", Some("launch.ks")));
        // Replayed from the save file after a load.
        wd.on_done("launch.ks", Some("old"));
        assert_eq!(wd.link().running.as_deref(), Some("launch.ks"));
        // Still watched: silence from here fails it.
        let failed = c.run(&mut wd, secs(11), 0.2);
        assert_eq!(failed.len(), 1);
    }

    #[test]
    fn rejection_before_first_beat_matches_unclaimed_entry() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        // Link down at dispatch, so the entry is unclaimed.
        wd.on_dispatch("launch.ks".into());
        wd.on_done("launch.ks", Some("a"));
        assert!(c.run(&mut wd, secs(20), 0.2).is_empty());
    }

    #[test]
    fn another_sessions_beats_do_not_keep_a_script_alive() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_heartbeat(beat("a", None));
        wd.on_dispatch("launch.ks".into());
        let mut failed = Vec::new();
        for _ in 0..12 {
            failed.extend(c.run(&mut wd, secs(1), 0.2));
            wd.on_heartbeat(beat("b", None));
        }
        assert_eq!(failed.len(), 1);
        assert!(wd.link().up);
    }

    #[test]
    fn vessel_switch_keeps_running_script_alive() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_heartbeat(beat("a", None));
        wd.on_dispatch("launch.ks".into());
        wd.on_heartbeat(beat("a", Some("launch.ks")));
        for _ in 0..30 {
            assert!(c.run(&mut wd, secs(1), 0.2).is_empty());
            wd.on_heartbeat(Heartbeat {
                boot: "a",
                path: Some("launch.ks"),
                active: false,
            });
            wd.on_heartbeat(beat("b", None));
        }
        assert_eq!(wd.link().running, None);
        wd.on_done("launch.ks", Some("a"));
    }

    #[test]
    fn unstamped_entry_adopts_first_active_boot() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_dispatch("launch.ks".into());
        assert!(c.run(&mut wd, secs(8), 0.2).is_empty());
        // A non-active CPU answering doesn't claim it.
        wd.on_heartbeat(Heartbeat {
            boot: "x",
            path: None,
            active: false,
        });
        wd.on_heartbeat(beat("a", Some("launch.ks")));
        for _ in 0..20 {
            assert!(c.run(&mut wd, secs(1), 0.2).is_empty());
            wd.on_heartbeat(beat("a", Some("launch.ks")));
        }
    }

    #[test]
    fn link_state_transitions() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        assert_eq!(*wd.link(), KosLink::default());
        wd.on_dispatch("launch.ks".into());
        assert!(!wd.link().up);
        wd.on_heartbeat(Heartbeat {
            boot: "x",
            path: Some("other.ks"),
            active: false,
        });
        assert!(!wd.link().up);
        wd.on_heartbeat(beat("a", Some("launch.ks")));
        assert_eq!(
            *wd.link(),
            KosLink {
                up: true,
                running: Some("launch.ks".into())
            }
        );
        c.run(&mut wd, secs(11), 0.2);
        assert_eq!(*wd.link(), KosLink::default());
    }

    #[test]
    fn ping_due_once_per_ut_advance_and_never_while_running() {
        let mut wd = ScriptWatchdog::new();
        assert!(!wd.ping_due());
        let mut c = Clock::new(&mut wd);
        assert!(wd.ping_due());
        assert!(!wd.ping_due());
        c.run(&mut wd, TICK, 0.2);
        assert!(wd.ping_due());
        wd.on_heartbeat(beat("a", Some("launch.ks")));
        c.run(&mut wd, TICK, 0.2);
        assert!(!wd.ping_due());
    }

    #[test]
    fn clean_finish_resumes_pings_and_keeps_link_up() {
        let mut wd = ScriptWatchdog::new();
        let mut c = Clock::new(&mut wd);
        wd.on_heartbeat(beat("a", None));
        wd.on_dispatch("launch.ks".into());
        wd.on_heartbeat(beat("a", Some("launch.ks")));
        wd.on_done("launch.ks", Some("a"));
        assert_eq!(wd.link().running, None);
        for _ in 0..15 {
            c.run(&mut wd, secs(1), 0.2);
            if wd.ping_due() {
                wd.on_heartbeat(beat("a", None));
            }
        }
        assert!(wd.link().up);
    }
}
