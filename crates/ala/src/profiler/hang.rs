use std::time::Duration;

use crate::scheduler::Instant;
use serde::Serialize;

pub const MEASUREMENT_VERSION: u32 = 2;

use super::SerializedLocation;
use super::journal::{
    ForegroundEvent, ForegroundJournal, ForegroundJournalCollector, ForegroundJournalEntry,
    FrameSnapshot, IntervalBoundary, IntervalSealer,
};

pub struct HangDetector {
    collector: ForegroundJournalCollector,
    sealer: IntervalSealer,
    threshold: Duration,
    frame_budget: Duration,
    first_present_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct HangIncident {
    pub snapshot: FrameSnapshot,
    pub trigger: HangTrigger,
    pub contributors: Vec<ForegroundEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HangTrigger {
    Threshold,
    Budget,
}

impl HangDetector {
    pub fn new(journal: ForegroundJournal, threshold: Duration, frame_budget: Duration) -> Self {
        Self {
            collector: journal.collector(),
            sealer: IntervalSealer::new(Instant::now()),
            threshold,
            frame_budget,
            first_present_at: None,
        }
    }

    pub fn first_present_at(&self) -> Option<Instant> {
        self.first_present_at
    }

    pub fn poll(&mut self) -> Vec<HangIncident> {
        let drained = self.collector.collect_unseen();
        if self.first_present_at.is_none() {
            self.first_present_at = drained.entries.iter().find_map(|entry| match entry {
                ForegroundJournalEntry::Boundary(IntervalBoundary::Presented(presented)) => {
                    Some(presented.presentation.present_end)
                }
                _ => None,
            });
        }
        self.sealer
            .push_entries(drained.entries)
            .into_iter()
            .filter_map(|snapshot| {
                HangIncident::detect(snapshot, self.threshold, self.frame_budget)
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SerializedHangIncident {
    pub measurement_version: u32,
    pub phase: &'static str,
    pub trigger: HangTrigger,
    pub start_ms: f64,
    pub active_ms: f64,
    pub stall_ms: f64,
    pub dirty_to_present_ms: Option<f64>,
    pub sealed_by: &'static str,
    pub busy_fraction: f64,
    pub event_count: usize,
    pub small_poll_count: u64,
    pub small_poll_total_ms: f64,
    pub dropped_events: u64,
    pub journal_discontinuous: bool,
    pub contributors: Vec<SerializedHangContributor>,
    pub contributors_elided: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SerializedHangContributor {
    TaskPoll {
        location: SerializedLocation,
        start_ms: f64,
        duration_ms: f64,
        depth: usize,
    },
    Action {
        name: &'static str,
        start_ms: f64,
        duration_ms: f64,
        depth: usize,
    },
    Input {
        input_kind: &'static str,
        start_ms: f64,
        duration_ms: f64,
        caused_invalidation: bool,
        depth: usize,
    },
    Draw {
        window_id: u64,
        start_ms: f64,
        duration_ms: f64,
        dirty_to_draw_ms: Option<f64>,
        invalidations: u64,
        depth: usize,
    },
    Present {
        window_id: u64,
        start_ms: f64,
        duration_ms: f64,
        depth: usize,
    },
}

fn as_millis(duration: Duration) -> f64 {
    duration.as_micros() as f64 / 1000.0
}

impl SerializedHangIncident {
    pub fn convert(
        startup: Instant,
        incident: &HangIncident,
        max_contributors: usize,
        first_present_at: Option<Instant>,
    ) -> Self {
        let since_startup =
            |instant: Instant| as_millis(instant.saturating_duration_since(startup));
        let snapshot = &incident.snapshot;
        let (active_start, active_end) = incident.active_window();
        let active = active_end.duration_since(active_start);
        let busy_fraction = if active.is_zero() {
            1.0
        } else {
            snapshot
                .occupancy_within(active_start, active_end)
                .div_duration_f64(active)
                .min(1.0)
        };
        Self {
            phase: match first_present_at {
                Some(first_present_at) if active_start >= first_present_at => "steady",
                _ => "startup",
            },
            measurement_version: MEASUREMENT_VERSION,
            trigger: incident.trigger,
            start_ms: since_startup(active_start),
            active_ms: as_millis(active),
            stall_ms: incident
                .contributors
                .first()
                .map(|event| as_millis(event.duration()))
                .unwrap_or(0.0),
            dirty_to_present_ms: match snapshot.boundary {
                IntervalBoundary::Presented(presented) => {
                    presented.dirty_to_present_duration().map(as_millis)
                }
                IntervalBoundary::Idle { .. } | IntervalBoundary::PowerTransition { .. } => None,
            },
            sealed_by: match snapshot.boundary {
                IntervalBoundary::Presented(_) => "present",
                IntervalBoundary::Idle { .. } => "idle",
                IntervalBoundary::PowerTransition { .. } => "power_transition",
            },
            busy_fraction: (busy_fraction * 1000.0).round() / 1000.0,
            event_count: snapshot.events.len(),
            small_poll_count: snapshot.small_poll_summary().count,
            small_poll_total_ms: as_millis(snapshot.small_poll_summary().total),
            dropped_events: snapshot.dropped_events,
            journal_discontinuous: snapshot.journal_discontinuous,
            contributors: {
                let mut kept: Vec<&ForegroundEvent> = incident
                    .contributors
                    .iter()
                    .take(max_contributors)
                    .collect();
                kept.sort_by_key(|event| event.start_time());
                kept.into_iter()
                    .map(|event| {
                        SerializedHangContributor::convert(
                            startup,
                            event,
                            nesting_depth(event, &snapshot.events),
                        )
                    })
                    .collect()
            },
            contributors_elided: incident.contributors.len().saturating_sub(max_contributors),
        }
    }
}

fn nesting_depth(event: &ForegroundEvent, events: &[ForegroundEvent]) -> usize {
    let (start, end) = (event.start_time(), event.end_time());
    events
        .iter()
        .filter(|other| {
            let (other_start, other_end) = (other.start_time(), other.end_time());
            other_start <= start && end <= other_end && (other_start < start || end < other_end)
        })
        .count()
}

impl SerializedHangContributor {
    fn convert(startup: Instant, event: &ForegroundEvent, depth: usize) -> Self {
        let since_startup =
            |instant: Instant| as_millis(instant.saturating_duration_since(startup));
        let duration_ms = as_millis(event.duration());
        match event {
            ForegroundEvent::TaskPoll(timing) => Self::TaskPoll {
                location: timing.location.into(),
                start_ms: since_startup(timing.start),
                duration_ms,
                depth,
            },
            ForegroundEvent::Action(timing) => Self::Action {
                name: timing.name,
                start_ms: since_startup(timing.start),
                duration_ms,
                depth,
            },
            ForegroundEvent::Input(timing) => Self::Input {
                input_kind: timing.kind,
                start_ms: since_startup(timing.start),
                duration_ms,
                caused_invalidation: timing.caused_invalidation,
                depth,
            },
            ForegroundEvent::Draw(timing) => Self::Draw {
                window_id: timing.window_id.as_u64(),
                start_ms: since_startup(timing.draw_start),
                duration_ms,
                dirty_to_draw_ms: timing.dirty_to_draw_duration().map(as_millis),
                invalidations: timing.invalidations,
                depth,
            },
            ForegroundEvent::Present(timing) => Self::Present {
                window_id: timing.window_id.as_u64(),
                start_ms: since_startup(timing.present_start),
                duration_ms,
                depth,
            },
            ForegroundEvent::SmallPolls(flush) => Self::TaskPoll {
                location: SerializedLocation {
                    file: "<small poll summary>".into(),
                    line: 0,
                    column: 0,
                },
                start_ms: since_startup(flush.since),
                duration_ms,
                depth,
            },
        }
    }
}

impl HangIncident {
    pub fn active_window(&self) -> (Instant, Instant) {
        let snapshot = &self.snapshot;
        let dirty_at = snapshot.boundary.dirty_at();
        let earliest_contributor = self
            .contributors
            .iter()
            .map(|event| event.start_time())
            .min();
        let start = [dirty_at, earliest_contributor]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(snapshot.interval_start);
        (start, snapshot.interval_end())
    }

    pub fn detect(
        snapshot: FrameSnapshot,
        threshold: Duration,
        frame_budget: Duration,
    ) -> Option<Self> {
        let mut contributors: Vec<ForegroundEvent> = snapshot
            .events
            .iter()
            .filter(|event| event.duration() >= threshold)
            .copied()
            .collect();
        let trigger = if contributors.is_empty() {
            if snapshot.journal_discontinuous {
                return None;
            }
            let spend = snapshot.occupancy_within(snapshot.interval_start, snapshot.interval_end());
            if spend < frame_budget {
                return None;
            }
            contributors = snapshot.events.clone();
            HangTrigger::Budget
        } else {
            HangTrigger::Threshold
        };
        contributors.sort_by_key(|event| std::cmp::Reverse(event.duration()));
        Some(Self {
            snapshot,
            trigger,
            contributors,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::thread;
    use std::time::Duration;

    use crate::scheduler::SpawnTime;
    use proptest::prelude::*;
    use rand::prelude::*;

    use crate::{
        self as ala, Context, FocusHandle, InteractiveElement, IntoElement, Modifiers, MouseButton,
        Render, Styled, TestAppContext, VisualTestContext, Window, WindowId, div, point, px,
    };

    use crate::profiler::{ActionTiming, FrameTiming, PresentTiming, TaskTiming, YieldTime};

    use super::super::journal::{
        FRAME_DEADLINE, ForegroundEvent, ForegroundJournalEntry, FrameSnapshot, FrameStateChange,
        InputTiming, IntervalBoundary, PollSummary, PresentedFrame, SmallPollFlush,
        install_test_foreground_journal, record_present,
    };
    use super::{
        HangDetector, HangIncident, HangTrigger, SerializedHangContributor, SerializedHangIncident,
    };

    actions!(hang_test, [HangyAction]);

    const HANG_THRESHOLD: Duration = Duration::from_millis(10);
    const FRAME_BUDGET: Duration = HANG_THRESHOLD;

    #[test]
    fn a_hang_outliving_the_frame_deadline_keeps_its_frame_association() {
        let start = crate::scheduler::Instant::now();
        let window_id = WindowId::from(0x51E17);
        let hang_end = start + FRAME_DEADLINE * 5;
        let presented_at = hang_end + Duration::from_millis(16);
        let (journal, _journal_guard) = install_test_foreground_journal(64, 8);
        let mut detector = HangDetector::new(journal, HANG_THRESHOLD, FRAME_BUDGET);

        let snapshots = detector.sealer.push_entries([
            ForegroundJournalEntry::FrameState(FrameStateChange::Pending {
                window_id,
                dirty_at: start,
            }),
            ForegroundJournalEntry::Event(task_poll_event(start, hang_end)),
        ]);
        assert!(snapshots.is_empty(), "nothing seals mid-hang");

        let snapshots = detector
            .sealer
            .push_entries([ForegroundJournalEntry::Boundary(
                IntervalBoundary::Presented(PresentedFrame {
                    frame: frame(window_id, presented_at),
                    presentation: PresentTiming {
                        window_id,
                        present_start: presented_at - Duration::from_millis(1),
                        present_end: presented_at,
                        animation_interval: None,
                    },
                }),
            )]);
        let [snapshot] = snapshots.as_slice() else {
            panic!("expected one presented snapshot, got {snapshots:?}");
        };
        let incident = HangIncident::detect(snapshot.clone(), HANG_THRESHOLD, FRAME_BUDGET)
            .expect("the hang qualifies");
        assert!(matches!(
            incident.contributors[0],
            ForegroundEvent::TaskPoll(timing) if timing.end.0 == hang_end
        ));
        assert!(matches!(
            incident.snapshot.boundary,
            IntervalBoundary::Presented(_)
        ));
    }

    #[test]
    fn serialized_incident_reports_presented_seal_fields() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);
        let window_id = WindowId::from(0xF1E1D);

        let presentation = PresentTiming {
            window_id,
            present_start: at(380),
            present_end: at(400),
            animation_interval: None,
        };
        let frame = FrameTiming {
            window_id,
            dirty_at: Some(at(100)),
            invalidations: 3,
            draw_start: at(350),
            draw_end: at(380),
        };
        let snapshot = FrameSnapshot {
            interval_start: at(150),
            boundary: IntervalBoundary::Presented(PresentedFrame {
                frame,
                presentation,
            }),
            events: vec![
                task_poll_event(at(150), at(300)),
                ForegroundEvent::Action(ActionTiming {
                    name: "test::SlowAction",
                    start: at(300),
                    end: at(340),
                }),
                ForegroundEvent::Input(InputTiming {
                    kind: "test",
                    start: at(340),
                    end: at(345),
                    caused_invalidation: false,
                }),
                ForegroundEvent::Present(presentation),
            ],
            small_polls: vec![SmallPollFlush {
                summary: PollSummary {
                    count: 2,
                    total: Duration::from_millis(1),
                },
                since: at(105),
                until: at(145),
            }],
            dropped_events: 0,
            journal_discontinuous: false,
        };

        let incident =
            HangIncident::detect(snapshot, HANG_THRESHOLD, FRAME_BUDGET).expect("has contributors");
        let serialized = SerializedHangIncident::convert(startup, &incident, 1, Some(at(50)));

        assert_eq!(serialized.phase, "steady");
        assert_eq!(serialized.trigger, HangTrigger::Threshold);
        assert_eq!(serialized.start_ms, 100.0);
        assert_eq!(serialized.active_ms, 300.0);
        assert_eq!(serialized.stall_ms, 150.0);
        assert_eq!(serialized.dirty_to_present_ms, Some(300.0));
        assert_eq!(serialized.sealed_by, "present");
        assert_eq!(serialized.busy_fraction, 0.72);
        assert_eq!(serialized.event_count, 4);
        assert_eq!(serialized.small_poll_count, 2);
        assert_eq!(serialized.small_poll_total_ms, 1.0);
        assert_eq!(serialized.dropped_events, 0);
        assert_eq!(serialized.contributors.len(), 1);
        assert_eq!(serialized.contributors_elided, 2);
        assert!(matches!(
            serialized.contributors[0],
            SerializedHangContributor::TaskPoll {
                start_ms,
                duration_ms,
                ..
            } if start_ms == 150.0 && duration_ms == 150.0
        ));
    }

    #[test]
    fn serialized_incident_reports_idle_seal_fields() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);

        let idle = FrameSnapshot {
            interval_start: at(200),
            boundary: IntervalBoundary::Idle { ended_at: at(260) },
            events: vec![task_poll_event(at(200), at(260))],
            small_polls: Vec::new(),
            dropped_events: 0,
            journal_discontinuous: false,
        };
        let incident =
            HangIncident::detect(idle, HANG_THRESHOLD, FRAME_BUDGET).expect("has contributors");
        let serialized = SerializedHangIncident::convert(startup, &incident, 8, None);
        assert_eq!(serialized.phase, "startup");
        assert_eq!(serialized.sealed_by, "idle");
        assert_eq!(serialized.dirty_to_present_ms, None);
        assert_eq!(serialized.start_ms, 200.0);
        assert_eq!(serialized.active_ms, 60.0);
        assert_eq!(serialized.stall_ms, 60.0);
        assert_eq!(serialized.busy_fraction, 1.0);
    }

    #[test]
    fn phase_is_startup_until_the_first_present() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);
        let snapshot = FrameSnapshot {
            interval_start: at(500),
            boundary: IntervalBoundary::Idle { ended_at: at(600) },
            events: vec![task_poll_event(at(500), at(600))],
            small_polls: Vec::new(),
            dropped_events: 0,
            journal_discontinuous: false,
        };
        let incident =
            HangIncident::detect(snapshot, HANG_THRESHOLD, FRAME_BUDGET).expect("has contributors");

        let phase = |first_present_at| {
            SerializedHangIncident::convert(startup, &incident, 8, first_present_at).phase
        };
        assert_eq!(phase(None), "startup", "nothing has been presented yet");
        assert_eq!(
            phase(Some(at(700))),
            "startup",
            "the incident began before the first presentation"
        );
        assert_eq!(
            phase(Some(at(500))),
            "steady",
            "an incident beginning exactly at first presentation is steady"
        );
        assert_eq!(phase(Some(at(100))), "steady");
    }

    #[test]
    fn first_present_at_latches_on_the_first_observed_presentation() {
        let (journal, _journal_guard) = install_test_foreground_journal(64, 8);
        let mut detector = HangDetector::new(journal, HANG_THRESHOLD, FRAME_BUDGET);
        let window_id = WindowId::from(0x1A7C4);

        let first_present_end = crate::scheduler::Instant::now();
        record_present(
            presentation(window_id, first_present_end),
            Some(frame(window_id, first_present_end)),
        );
        detector.poll();
        let latched = detector
            .first_present_at()
            .expect("a presentation was recorded");
        assert_eq!(latched, first_present_end);

        record_present(
            presentation(window_id, first_present_end + Duration::from_millis(16)),
            Some(frame(
                window_id,
                first_present_end + Duration::from_millis(16),
            )),
        );
        detector.poll();
        assert_eq!(detector.first_present_at(), Some(latched));
    }

    #[test]
    fn busy_fraction_excludes_small_polls_outside_the_active_window() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);
        let snapshot = FrameSnapshot {
            interval_start: at(0),
            boundary: IntervalBoundary::Idle { ended_at: at(1000) },
            events: vec![task_poll_event(at(900), at(950))],
            small_polls: vec![SmallPollFlush {
                summary: PollSummary {
                    count: 500,
                    total: Duration::from_millis(400),
                },
                since: at(0),
                until: at(800),
            }],
            dropped_events: 0,
            journal_discontinuous: false,
        };

        let incident =
            HangIncident::detect(snapshot, HANG_THRESHOLD, FRAME_BUDGET).expect("has contributors");
        let serialized = SerializedHangIncident::convert(startup, &incident, 8, None);

        assert_eq!(serialized.busy_fraction, 0.5);
    }

    #[test]
    fn an_interval_of_small_work_over_budget_is_an_incident() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);
        let window_id = WindowId::from(0xB0D6E7);
        let snapshot = FrameSnapshot {
            interval_start: at(0),
            boundary: IntervalBoundary::Presented(PresentedFrame {
                frame: FrameTiming {
                    window_id,
                    dirty_at: Some(at(0)),
                    invalidations: 1,
                    draw_start: at(140),
                    draw_end: at(145),
                },
                presentation: PresentTiming {
                    window_id,
                    present_start: at(145),
                    present_end: at(150),
                    animation_interval: None,
                },
            }),
            events: vec![
                task_poll_event(at(0), at(5)),
                task_poll_event(at(5), at(13)),
                task_poll_event(at(13), at(18)),
            ],
            small_polls: Vec::new(),
            dropped_events: 0,
            journal_discontinuous: false,
        };

        let incident = HangIncident::detect(snapshot, HANG_THRESHOLD, FRAME_BUDGET)
            .expect("foreground spend exceeded the frame budget");
        assert_eq!(incident.trigger, HangTrigger::Budget);
        assert_eq!(incident.contributors.len(), 3);
        assert_eq!(
            incident.contributors[0].duration(),
            Duration::from_millis(8)
        );
        let serialized = SerializedHangIncident::convert(startup, &incident, 8, Some(startup));
        assert_eq!(serialized.trigger, HangTrigger::Budget);
        assert_eq!(serialized.stall_ms, 8.0);
        assert_eq!(serialized.dirty_to_present_ms, Some(150.0));
        assert_eq!(serialized.sealed_by, "present");
    }

    #[test]
    fn a_journal_gap_suppresses_budget_inference_but_retains_observed_hangs() {
        let start = crate::scheduler::Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut sealer = super::super::journal::IntervalSealer::new(start);
        let snapshots = sealer.push_entries([
            ForegroundJournalEntry::Event(task_poll_event(at(0), at(5))),
            ForegroundJournalEntry::Discontinuity { lost: 1 },
            ForegroundJournalEntry::Event(task_poll_event(at(10), at(15))),
            ForegroundJournalEntry::Boundary(IntervalBoundary::Idle { ended_at: at(15) }),
            ForegroundJournalEntry::Discontinuity { lost: 1 },
            ForegroundJournalEntry::Event(task_poll_event(at(20), at(32))),
            ForegroundJournalEntry::Boundary(IntervalBoundary::Idle { ended_at: at(32) }),
            ForegroundJournalEntry::Event(task_poll_event(at(40), at(52))),
            ForegroundJournalEntry::Boundary(IntervalBoundary::Idle { ended_at: at(52) }),
        ]);
        let [budget_only, observed_hang, clean] = snapshots.as_slice() else {
            panic!("expected two discontinuous snapshots followed by a clean one");
        };

        assert!(budget_only.journal_discontinuous);
        assert_eq!(budget_only.dropped_events, 1);
        assert!(HangIncident::detect(budget_only.clone(), HANG_THRESHOLD, FRAME_BUDGET).is_none());

        let observed_hang =
            HangIncident::detect(observed_hang.clone(), HANG_THRESHOLD, FRAME_BUDGET)
                .expect("the directly observed threshold-qualified hang remains valid");
        assert!(observed_hang.snapshot.journal_discontinuous);
        assert_eq!(observed_hang.contributors.len(), 1);
        let serialized = SerializedHangIncident::convert(start, &observed_hang, 8, None);
        assert!(serialized.journal_discontinuous);

        assert!(!clean.journal_discontinuous);
        assert!(HangIncident::detect(clean.clone(), HANG_THRESHOLD, FRAME_BUDGET).is_some());
    }

    #[test]
    fn a_slow_frame_with_little_foreground_spend_is_not_an_incident() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);
        let window_id = WindowId::from(0xFA57);
        let snapshot = FrameSnapshot {
            interval_start: at(0),
            boundary: IntervalBoundary::Presented(PresentedFrame {
                frame: FrameTiming {
                    window_id,
                    dirty_at: Some(at(0)),
                    invalidations: 1,
                    draw_start: at(145),
                    draw_end: at(147),
                },
                presentation: PresentTiming {
                    window_id,
                    present_start: at(149),
                    present_end: at(150),
                    animation_interval: None,
                },
            }),
            events: vec![task_poll_event(at(0), at(5))],
            small_polls: Vec::new(),
            dropped_events: 0,
            journal_discontinuous: false,
        };

        assert!(HangIncident::detect(snapshot, HANG_THRESHOLD, FRAME_BUDGET).is_none());
    }

    #[test]
    fn an_idle_sealed_interval_over_budget_is_an_incident() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);
        let snapshot = FrameSnapshot {
            interval_start: at(0),
            boundary: IntervalBoundary::Idle { ended_at: at(200) },
            events: (0..10)
                .map(|i| task_poll_event(at(i * 20), at(i * 20 + 5)))
                .collect(),
            small_polls: Vec::new(),
            dropped_events: 0,
            journal_discontinuous: false,
        };

        let incident = HangIncident::detect(snapshot, HANG_THRESHOLD, FRAME_BUDGET)
            .expect("foreground spend exceeded the frame budget");
        assert_eq!(incident.contributors.len(), 10);
        let serialized = SerializedHangIncident::convert(startup, &incident, 8, Some(startup));
        assert_eq!(serialized.sealed_by, "idle");
        assert_eq!(serialized.dirty_to_present_ms, None);
        assert_eq!(serialized.stall_ms, 5.0);
        assert_eq!(serialized.contributors_elided, 2);
    }

    #[test]
    fn serialized_contributors_are_chronological_with_nesting_depths() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);
        let window_id = WindowId::from(0x2E57ED);
        let snapshot = FrameSnapshot {
            interval_start: at(0),
            boundary: IntervalBoundary::Idle { ended_at: at(80) },
            events: vec![
                task_poll_event(at(50), at(70)),
                ForegroundEvent::Input(InputTiming {
                    kind: "mouse_move",
                    start: at(0),
                    end: at(40),
                    caused_invalidation: true,
                }),
                ForegroundEvent::Draw(FrameTiming {
                    window_id,
                    dirty_at: Some(at(0)),
                    invalidations: 1,
                    draw_start: at(1),
                    draw_end: at(39),
                }),
            ],
            small_polls: Vec::new(),
            dropped_events: 0,
            journal_discontinuous: false,
        };

        let incident =
            HangIncident::detect(snapshot, HANG_THRESHOLD, FRAME_BUDGET).expect("has contributors");
        let serialized = SerializedHangIncident::convert(startup, &incident, 8, Some(startup));

        assert_eq!(serialized.stall_ms, 40.0);
        assert!(matches!(
            serialized.contributors.as_slice(),
            [
                SerializedHangContributor::Input {
                    input_kind: "mouse_move",
                    depth: 0,
                    ..
                },
                SerializedHangContributor::Draw { depth: 1, .. },
                SerializedHangContributor::TaskPoll { depth: 0, .. },
            ]
        ));
    }

    #[test]
    fn small_poll_spend_alone_can_reach_the_budget() {
        let startup = crate::scheduler::Instant::now();
        let at = |ms: u64| startup + Duration::from_millis(ms);
        let window_id = WindowId::from(0xDE1A7);
        let snapshot = FrameSnapshot {
            interval_start: at(0),
            boundary: IntervalBoundary::Presented(PresentedFrame {
                frame: FrameTiming {
                    window_id,
                    dirty_at: Some(at(0)),
                    invalidations: 1,
                    draw_start: at(148),
                    draw_end: at(149),
                },
                presentation: PresentTiming {
                    window_id,
                    present_start: at(149),
                    present_end: at(150),
                    animation_interval: None,
                },
            }),
            events: Vec::new(),
            small_polls: vec![SmallPollFlush {
                summary: PollSummary {
                    count: 40,
                    total: Duration::from_millis(12),
                },
                since: at(0),
                until: at(150),
            }],
            dropped_events: 0,
            journal_discontinuous: false,
        };

        let incident = HangIncident::detect(snapshot, HANG_THRESHOLD, FRAME_BUDGET)
            .expect("small-poll spend exceeded the frame budget");
        assert!(incident.contributors.is_empty());
        let serialized = SerializedHangIncident::convert(startup, &incident, 8, Some(startup));
        assert_eq!(serialized.stall_ms, 0.0);
        assert_eq!(serialized.event_count, 0);
        assert_eq!(serialized.small_poll_count, 40);
        assert_eq!(serialized.dirty_to_present_ms, Some(150.0));
        assert_eq!(serialized.busy_fraction, 0.08);
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        #[test]
        fn detection_matches_threshold_and_budget_model_at_boundaries(
            durations_ms in prop::collection::vec(0u16..=50, 0..=16),
            threshold_ms in 0u16..=60,
            frame_budget_ms in 0u16..=500,
            small_poll_total_ms in 0u16..=100,
        ) {
            let origin = crate::scheduler::Instant::now();
            let mut cursor_ms = 0u64;
            let events = durations_ms
                .iter()
                .map(|duration_ms| {
                    let start = origin + Duration::from_millis(cursor_ms);
                    cursor_ms += u64::from(*duration_ms);
                    let end = origin + Duration::from_millis(cursor_ms);
                    cursor_ms += 1;
                    ForegroundEvent::Input(InputTiming {
                        kind: "test",
                        start,
                        end,
                        caused_invalidation: false,
                    })
                })
                .collect::<Vec<_>>();
            let interval_end_ms = cursor_ms.max(1);
            let snapshot = FrameSnapshot {
                interval_start: origin,
                boundary: IntervalBoundary::Idle {
                    ended_at: origin + Duration::from_millis(interval_end_ms),
                },
                events,
                small_polls: vec![SmallPollFlush {
                    summary: PollSummary {
                        count: u64::from(small_poll_total_ms > 0),
                        total: Duration::from_millis(u64::from(small_poll_total_ms)),
                    },
                    since: origin,
                    until: origin + Duration::from_millis(interval_end_ms),
                }],
                dropped_events: 0,
                journal_discontinuous: false,
            };
            let qualifying_durations = durations_ms
                .iter()
                .copied()
                .filter(|duration| *duration >= threshold_ms)
                .collect::<Vec<_>>();
            let occupancy_ms = durations_ms
                .iter()
                .map(|duration| u64::from(*duration))
                .sum::<u64>()
                + u64::from(small_poll_total_ms);
            let expected_incident =
                !qualifying_durations.is_empty() || occupancy_ms >= u64::from(frame_budget_ms);

            let incident = HangIncident::detect(
                snapshot,
                Duration::from_millis(u64::from(threshold_ms)),
                Duration::from_millis(u64::from(frame_budget_ms)),
            );
            prop_assert_eq!(incident.is_some(), expected_incident);

            if let Some(incident) = incident {
                let mut expected_contributors = if qualifying_durations.is_empty() {
                    durations_ms
                } else {
                    qualifying_durations
                };
                expected_contributors.sort_unstable_by(|first, second| second.cmp(first));
                let observed_contributors = incident
                    .contributors
                    .iter()
                    .map(|event| event.duration().as_millis() as u16)
                    .collect::<Vec<_>>();
                prop_assert_eq!(observed_contributors, expected_contributors);
            }
        }
    }

    #[derive(Clone, Default)]
    struct HangControls {
        render: Rc<Cell<Option<Duration>>>,
        input: Rc<Cell<Option<Duration>>>,
        action: Rc<Cell<Option<Duration>>>,
    }

    struct HangyView {
        controls: HangControls,
        focus_handle: FocusHandle,
    }

    impl Render for HangyView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if let Some(duration) = self.controls.render.take() {
                thread::sleep(duration);
            }
            let action_controls = self.controls.clone();
            let input_controls = self.controls.clone();
            div()
                .size_full()
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(move |_, _: &HangyAction, _, _| {
                    if let Some(duration) = action_controls.action.take() {
                        thread::sleep(duration);
                    }
                }))
                .on_mouse_down(MouseButton::Left, move |_, _, _| {
                    if let Some(duration) = input_controls.input.take() {
                        thread::sleep(duration);
                    }
                })
        }
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    enum HangKind {
        Render,
        Input,
        Action,
        Poll,
    }

    #[ala::test(iterations = 5)]
    fn detects_randomly_placed_foreground_hangs(mut rng: StdRng, cx: &mut TestAppContext) {
        let (journal, _journal_guard) = install_test_foreground_journal(1024, 16);
        let mut detector = HangDetector::new(journal, HANG_THRESHOLD, FRAME_BUDGET);

        let controls = HangControls::default();
        let (view, cx) = cx.add_window_view(|_, cx| HangyView {
            controls: controls.clone(),
            focus_handle: cx.focus_handle(),
        });
        view.update_in(cx, |view, window, cx| {
            window.focus(&view.focus_handle, cx);
        });
        draw_window(cx);

        let mut injected: Vec<(HangKind, Duration)> = Vec::new();
        for _ in 0..rng.random_range(1..=4) {
            let duration = HANG_THRESHOLD + Duration::from_millis(rng.random_range(5..25));
            let kind = match rng.random_range(0..4) {
                0 => {
                    controls.render.set(Some(duration));
                    view.update_in(cx, |_, _, cx| cx.notify());
                    draw_window(cx);
                    HangKind::Render
                }
                1 => {
                    controls.input.set(Some(duration));
                    cx.simulate_mouse_down(
                        point(px(5.), px(5.)),
                        MouseButton::Left,
                        Modifiers::none(),
                    );
                    HangKind::Input
                }
                2 => {
                    controls.action.set(Some(duration));
                    cx.dispatch_action(HangyAction);
                    HangKind::Action
                }
                _ => {
                    simulate_blocked_foreground_poll(duration);
                    HangKind::Poll
                }
            };
            injected.push((kind, duration));

            if rng.random_bool(0.5) {
                draw_window(cx);
            }
            if rng.random_bool(0.5) {
                cx.simulate_mouse_up(point(px(5.), px(5.)), MouseButton::Left, Modifiers::none());
            }
        }

        draw_window(cx);

        let incidents = detector.poll();
        let contributors: Vec<ForegroundEvent> = incidents
            .iter()
            .flat_map(|incident| incident.contributors.iter().copied())
            .collect();

        for kind in [
            HangKind::Render,
            HangKind::Input,
            HangKind::Action,
            HangKind::Poll,
        ] {
            let expected: Vec<Duration> = injected
                .iter()
                .filter(|(injected_kind, _)| *injected_kind == kind)
                .map(|(_, duration)| *duration)
                .collect();
            let observed: Vec<Duration> = contributors
                .iter()
                .filter(|event| matches_kind(event, kind) && event.duration() >= HANG_THRESHOLD)
                .map(|event| event.duration())
                .collect();
            assert_all_matched(kind, expected, observed);
        }
    }

    fn assert_all_matched(
        kind: HangKind,
        mut expected: Vec<Duration>,
        mut observed: Vec<Duration>,
    ) {
        assert_eq!(
            observed.len(),
            expected.len(),
            "expected every observed {kind:?} hang to correspond to one injection; \
             expected {expected:?}, observed {observed:?}"
        );
        expected.sort_unstable_by(|a, b| b.cmp(a));
        observed.sort_unstable_by(|a, b| b.cmp(a));
        let mut observed = observed.into_iter();
        for expected_duration in expected {
            let matched = observed.find(|observed| *observed >= expected_duration);
            assert!(
                matched.is_some(),
                "injected {kind:?} hang of {expected_duration:?} was not detected"
            );
        }
    }

    fn matches_kind(event: &ForegroundEvent, kind: HangKind) -> bool {
        match (event, kind) {
            (ForegroundEvent::Draw(_), HangKind::Render) => true,
            (ForegroundEvent::Input(_), HangKind::Input) => true,
            (ForegroundEvent::Action(timing), HangKind::Action) => {
                timing.name.ends_with("HangyAction")
            }
            (ForegroundEvent::TaskPoll(timing), HangKind::Poll) => {
                timing.location.file() == file!()
            }
            _ => false,
        }
    }

    fn draw_window(cx: &mut VisualTestContext) {
        cx.update(|window, cx| {
            let arena_clear = window.draw(cx);
            window.present_if_needed();
            arena_clear.clear(cx);
        });
    }

    fn simulate_blocked_foreground_poll(duration: Duration) {
        let location = std::panic::Location::caller();
        crate::profiler::update_running_task(SpawnTime(crate::scheduler::Instant::now()), location);
        thread::sleep(duration);
        crate::profiler::save_task_timing();
    }

    fn task_poll_event(
        start: crate::scheduler::Instant,
        end: crate::scheduler::Instant,
    ) -> ForegroundEvent {
        ForegroundEvent::TaskPoll(TaskTiming {
            location: std::panic::Location::caller(),
            spawned: SpawnTime(start),
            start,
            end: YieldTime(end),
        })
    }

    fn presentation(window_id: WindowId, present_end: crate::scheduler::Instant) -> PresentTiming {
        PresentTiming {
            window_id,
            present_start: present_end - Duration::from_millis(1),
            present_end,
            animation_interval: None,
        }
    }

    fn frame(window_id: WindowId, draw_end: crate::scheduler::Instant) -> FrameTiming {
        FrameTiming {
            window_id,
            dirty_at: Some(draw_end - Duration::from_millis(2)),
            invalidations: 1,
            draw_start: draw_end - Duration::from_millis(1),
            draw_end,
        }
    }
}
