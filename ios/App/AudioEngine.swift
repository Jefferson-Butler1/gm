import AVFoundation

/// Plays what Rust decides each frame (`HudData.sounds` and `.mood`).
///
/// SFX: every sound preloaded as a PCM buffer, played on a small round-robin pool of
/// player nodes, each through a varispeed for a little pitch variation on the repeated
/// ones. Music: two looping tracks that play in sync and crossfade on mood change.
///
/// The session is `.ambient`: the silent switch mutes the game, and other apps' audio (a
/// podcast, Music) keeps playing underneath rather than being interrupted.
final class AudioEngine {
    /// Bundle file, gain, and whether repeats vary in pitch. Every `.caf` is mono 44.1 kHz
    /// (see ASSETS.md), so all voices share one format.
    private static let sfx: [Sound: (file: String, gain: Float, vary: Bool)] = [
        .playerShot: ("player_shot", 0.45, true),
        .enemyShot: ("enemy_shot", 0.35, true),
        .enemyHit: ("enemy_hit", 0.6, true),
        .enemyKilled: ("enemy_killed", 0.7, true),
        .enemyAlerted: ("enemy_alerted", 0.5, false),
        .playerHurt: ("player_hurt", 0.9, false),
        .playerDied: ("player_died", 1, false),
        .pitFall: ("pit_fall", 0.6, false),
        .roll: ("roll", 0.5, true),
        .ventStart: ("vent_start", 0.5, false),
        .ventDone: ("vent_done", 0.45, false),
        .hatchOpened: ("hatch_opened", 0.6, false),
        .roomSealed: ("room_sealed", 0.8, false),
        .waveStarted: ("wave_started", 0.5, false),
        .roomCleared: ("room_cleared", 0.6, false),
        .won: ("won", 0.9, false),
    ]
    private static let voiceCount = 12
    private static let crossfadeSecs: TimeInterval = 1.5

    private let engine = AVAudioEngine()
    private var voices: [(player: AVAudioPlayerNode, varispeed: AVAudioUnitVarispeed)] = []
    private var nextVoice = 0
    private var buffers: [Sound: AVAudioPCMBuffer] = [:]
    private let music: [Mood: AVAudioPlayer]
    private var mood = Mood.explore
    private var musicVolume: Float = 1

    init() {
        try? AVAudioSession.sharedInstance().setCategory(.ambient)
        try? AVAudioSession.sharedInstance().setActive(true)

        for (sound, entry) in Self.sfx {
            guard let url = Bundle.main.url(forResource: entry.file, withExtension: "caf"),
                  let file = try? AVAudioFile(forReading: url),
                  let buffer = AVAudioPCMBuffer(pcmFormat: file.processingFormat,
                                                frameCapacity: AVAudioFrameCount(file.length)),
                  (try? file.read(into: buffer)) != nil
            else {
                fputs("[gm] audio: missing \(entry.file).caf\n", stderr)
                continue
            }
            buffers[sound] = buffer
        }
        if let format = buffers.values.first?.format {
            for _ in 0..<Self.voiceCount {
                let player = AVAudioPlayerNode()
                let varispeed = AVAudioUnitVarispeed()
                engine.attach(player)
                engine.attach(varispeed)
                engine.connect(player, to: varispeed, format: format)
                engine.connect(varispeed, to: engine.mainMixerNode, format: format)
                voices.append((player, varispeed))
            }
        }

        var music: [Mood: AVAudioPlayer] = [:]
        for (mood, file) in [(Mood.explore, "music_explore"), (.combat, "music_combat")] {
            guard let url = Bundle.main.url(forResource: file, withExtension: "m4a"),
                  let player = try? AVAudioPlayer(contentsOf: url)
            else {
                fputs("[gm] audio: missing \(file).m4a\n", stderr)
                continue
            }
            player.numberOfLoops = -1
            player.volume = 0
            player.prepareToPlay()
            music[mood] = player
        }
        self.music = music
        try? engine.start()
    }

    /// Volumes in 0...1; apply live.
    func setVolumes(master: Float, music: Float, sfx: Float) {
        engine.mainMixerNode.outputVolume = master * sfx
        musicVolume = master * music
        fadeMusic(duration: 0)
    }

    /// Called every display-link frame with Rust's picks.
    func frame(sounds: [Sound], mood: Mood) {
        if mood != self.mood {
            self.mood = mood
            fadeMusic(duration: Self.crossfadeSecs)
        }
        // Both loops run the whole time, in sync; the fade picks which one is heard. An
        // interruption (a call, backgrounding) stops them, so this also restarts them.
        for player in music.values where !player.isPlaying {
            player.play()
        }
        guard !sounds.isEmpty, !voices.isEmpty else { return }
        if !engine.isRunning {
            do {
                try engine.start()
            } catch {
                return
            }
        }
        for sound in sounds {
            guard let buffer = buffers[sound], let entry = Self.sfx[sound] else { continue }
            // Round-robin: with every voice busy, the oldest sound is cut off.
            let voice = voices[nextVoice]
            nextVoice = (nextVoice + 1) % voices.count
            voice.player.stop()
            voice.player.volume = entry.gain
            voice.varispeed.rate = entry.vary ? Float.random(in: 0.94...1.06) : 1
            voice.player.scheduleBuffer(buffer)
            voice.player.play()
        }
    }

    private func fadeMusic(duration: TimeInterval) {
        for (mood, player) in music {
            player.setVolume(mood == self.mood ? musicVolume : 0, fadeDuration: duration)
        }
    }
}
