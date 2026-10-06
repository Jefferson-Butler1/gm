//! Which haptics to play. Rust decides: each sim step's events for slot 0, plus
//! transitions read off the states either side of it (rolls, vents, a room sealing),
//! become [`Haptic`]s collected for the frame; Swift plays them. A frame plays at most
//! [`PER_FRAME`], strongest first, so a volley of hits doesn't turn into mush.

use sim::{Event, Run, SimState};

/// Haptics played per frame, at most.
const PER_FRAME: usize = 2;
/// The player's own shots tick at most this often, in seconds, whatever the fire rate.
const SHOT_INTERVAL_SECS: f64 = 0.1;

/// Declared weakest to strongest: a frame keeps the strongest [`PER_FRAME`].
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Haptic {
    /// Slot 0 fired (rate-limited).
    Shot,
    EnemyHit,
    Roll,
    /// The phase pistol started venting (emptied, or vented by hand).
    VentStart,
    /// The vent finished: charges are back.
    VentDone,
    EnemyKilled,
    /// A reinforcement wave spawned.
    WaveStart,
    /// An encounter started: the room's hatches sealed.
    HatchSeal,
    /// An EMP went off.
    Emp,
    /// Slot 0 took a hit or fell into a pit.
    PlayerHurt,
    Won,
    PlayerDied,
}

#[derive(Default)]
pub struct Haptics {
    pending: Vec<Haptic>,
    /// Display time of the last shot tick played.
    last_shot: Option<f64>,
}

impl Haptics {
    /// One sim step from `prev` to `current`, which emitted `events`.
    pub fn note(&mut self, prev: &SimState, current: &SimState, events: &[Event]) {
        // A restart swaps in a fresh run: `prev` and `current` aren't one step apart.
        if events.contains(&Event::Restarted) {
            return;
        }
        for event in events {
            let haptic = match *event {
                Event::ShotFired { slot: 0 } => Haptic::Shot,
                Event::EnemyHit { .. } => Haptic::EnemyHit,
                Event::EnemyKilled { .. } => Haptic::EnemyKilled,
                Event::PlayerHit { slot: 0 } | Event::PlayerFell { slot: 0 } => Haptic::PlayerHurt,
                Event::PlayerDied { slot: 0 } => Haptic::PlayerDied,
                Event::WaveStarted { .. } => Haptic::WaveStart,
                Event::EmpDetonated { .. } => Haptic::Emp,
                Event::Won => Haptic::Won,
                _ => continue,
            };
            self.pending.push(haptic);
        }
        if let (Some(before), Some(after)) = (prev.players[0], current.players[0]) {
            // A roll starts at the full `roll_ticks` and only counts down from there.
            let roll_ticks = current.config.tuning.roll_ticks;
            if after.roll_ticks == roll_ticks && before.roll_ticks != roll_ticks {
                self.pending.push(Haptic::Roll);
            }
            match (before.gun.venting(), after.gun.venting()) {
                (false, true) => self.pending.push(Haptic::VentStart),
                (true, false) => self.pending.push(Haptic::VentDone),
                _ => {}
            }
        }
        if prev.run == Run::Boarding && matches!(current.run, Run::Encounter { .. }) {
            self.pending.push(Haptic::HatchSeal);
        }
    }

    /// The frame's haptics at display time `now`, strongest first, clearing the rest.
    pub fn take(&mut self, now: f64) -> Vec<Haptic> {
        let mut haptics = std::mem::take(&mut self.pending);
        haptics.sort_unstable_by(|a, b| b.cmp(a));
        haptics.dedup();
        haptics.truncate(PER_FRAME);
        if haptics.contains(&Haptic::Shot) {
            if self
                .last_shot
                .is_some_and(|last| now - last < SHOT_INTERVAL_SECS)
            {
                haptics.retain(|&h| h != Haptic::Shot);
            } else {
                self.last_shot = Some(now);
            }
        }
        haptics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::RunConfig;

    #[test]
    fn a_frame_keeps_its_two_strongest_once_each() {
        let prev = SimState::new(1, RunConfig::default());
        let mut current = prev.clone();
        current.players[0].as_mut().expect("slot 0").roll_ticks = current.config.tuning.roll_ticks;
        current.run = Run::Encounter {
            room: sim::RoomId(0),
            wave: 0,
        };
        let mut haptics = Haptics::default();
        let events = [
            Event::ShotFired { slot: 0 },
            Event::PlayerHit { slot: 1 }, // not ours
            Event::WaveStarted { wave: 1 },
            Event::WaveStarted { wave: 1 },
        ];
        haptics.note(&prev, &current, &events);
        assert_eq!(haptics.take(0.0), [Haptic::HatchSeal, Haptic::WaveStart]);
        // Mid-roll, fight still on: nothing new.
        haptics.note(&current, &current, &[]);
        assert_eq!(haptics.take(0.0), []);
    }

    #[test]
    fn shots_tick_at_most_every_interval() {
        let state = SimState::new(1, RunConfig::default());
        let mut haptics = Haptics::default();
        let mut shot_at = |now| {
            haptics.note(&state, &state, &[Event::ShotFired { slot: 0 }]);
            haptics.take(now)
        };
        assert_eq!(shot_at(1.0), [Haptic::Shot]);
        assert_eq!(shot_at(1.05), []);
        assert_eq!(shot_at(1.1), [Haptic::Shot]);
    }
}
