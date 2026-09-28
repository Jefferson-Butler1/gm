//! What to play. Rust decides: each sim step's events, plus transitions read off the
//! states either side of it (rolls, vents, a room sealing), become [`Sound`]s collected
//! for the frame; Swift plays them. Each sound plays at most once per frame, so rapid
//! fire or a volley of hits doesn't stack copies.

use sim::{Event, Run, SimState};

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    PlayerShot,
    EnemyShot,
    EnemyHit,
    EnemyKilled,
    /// An unaware enemy noticed the party (the "!").
    EnemyAlerted,
    PlayerHurt,
    PlayerDied,
    PitFall,
    Roll,
    /// The phase pistol started venting (emptied, or vented by hand).
    VentStart,
    /// The vent finished: charges are back.
    VentDone,
    HatchOpened,
    /// An encounter started: the room's enemies spawned and its hatches sealed.
    RoomSealed,
    /// A reinforcement wave spawned.
    WaveStarted,
    RoomCleared,
    Won,
}

/// Which music loop fits the run.
#[derive(uniffi::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mood {
    #[default]
    Explore,
    /// A fight is on.
    Combat,
    /// Dead or extracted: music fades out under the stinger.
    Silent,
}

impl Mood {
    pub const fn of(run: Run) -> Self {
        match run {
            Run::Boarding => Self::Explore,
            Run::Encounter { .. } => Self::Combat,
            Run::Dead { .. } | Run::Won => Self::Silent,
        }
    }
}

#[derive(Default)]
pub struct Audio {
    sounds: Vec<Sound>,
}

impl Audio {
    /// One sim step from `prev` to `current`, which emitted `events`.
    pub fn note(&mut self, prev: &SimState, current: &SimState, events: &[Event]) {
        // A restart swaps in a fresh run: `prev` and `current` aren't one step apart.
        if events.contains(&Event::Restarted) {
            return;
        }
        for event in events {
            let sound = match event {
                Event::ShotFired { .. } => Sound::PlayerShot,
                Event::EnemyFired { .. } => Sound::EnemyShot,
                Event::EnemyHit { .. } => Sound::EnemyHit,
                Event::EnemyKilled { .. } => Sound::EnemyKilled,
                Event::EnemyAlerted { .. } => Sound::EnemyAlerted,
                Event::PlayerHit { .. } => Sound::PlayerHurt,
                Event::PlayerFell { .. } => Sound::PitFall,
                Event::PlayerDied { .. } => Sound::PlayerDied,
                Event::HatchOpened { .. } => Sound::HatchOpened,
                Event::WaveStarted { .. } => Sound::WaveStarted,
                Event::RoomCleared { .. } => Sound::RoomCleared,
                Event::Won => Sound::Won,
                Event::Restarted => continue,
            };
            self.add(sound);
        }
        let roll_ticks = current.config.tuning.roll_ticks;
        for (before, after) in prev.players.iter().zip(&current.players) {
            let (Some(before), Some(after)) = (before, after) else {
                continue;
            };
            // A roll starts at the full `roll_ticks` and only counts down from there.
            if after.roll_ticks == roll_ticks && before.roll_ticks != roll_ticks {
                self.add(Sound::Roll);
            }
            match (before.gun.venting(), after.gun.venting()) {
                (false, true) => self.add(Sound::VentStart),
                (true, false) => self.add(Sound::VentDone),
                _ => {}
            }
        }
        if prev.run == Run::Boarding && matches!(current.run, Run::Encounter { .. }) {
            self.add(Sound::RoomSealed);
        }
    }

    fn add(&mut self, sound: Sound) {
        if !self.sounds.contains(&sound) {
            self.sounds.push(sound);
        }
    }

    /// The frame's sounds, clearing them.
    pub fn take(&mut self) -> Vec<Sound> {
        let mut sounds = std::mem::take(&mut self.sounds);
        // The death sound covers the hit that killed.
        if sounds.contains(&Sound::PlayerDied) {
            sounds.retain(|&s| s != Sound::PlayerHurt);
        }
        sounds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::RunConfig;

    /// The transitions read off state, rather than events, are the fragile part.
    #[test]
    fn rolls_vents_and_sealing_come_from_state_and_play_once() {
        let prev = SimState::new(1, RunConfig::default());
        let mut current = prev.clone();
        let p = current.players[0].as_mut().expect("slot 0");
        p.roll_ticks = current.config.tuning.roll_ticks;
        p.gun.vent_ticks = 10;
        current.run = Run::Encounter {
            room: sim::RoomId(0),
            wave: 0,
        };
        let mut audio = Audio::default();
        let hits = [Event::PlayerHit { slot: 0 }, Event::PlayerHit { slot: 0 }];
        audio.note(&prev, &current, &hits);
        audio.note(&prev, &current, &hits);
        assert_eq!(
            audio.take(),
            [
                Sound::PlayerHurt,
                Sound::Roll,
                Sound::VentStart,
                Sound::RoomSealed
            ]
        );

        // Mid-roll, still venting: nothing new. Then the vent ends.
        let mut later = current.clone();
        audio.note(&current, &later, &[]);
        assert_eq!(audio.take(), []);
        later.players[0].as_mut().expect("slot 0").gun.vent_ticks = 0;
        let death = [Event::PlayerHit { slot: 0 }, Event::PlayerDied { slot: 0 }];
        audio.note(&current, &later, &death);
        assert_eq!(audio.take(), [Sound::PlayerDied, Sound::VentDone]);
    }
}
