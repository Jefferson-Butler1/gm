import SwiftUI

@main
struct GMApp: App {
    @State private var model = GameModel()

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
        }
    }
}

@Observable
final class GameModel {
    /// Controls setting options, with stable keys for persistence.
    static let schemes: [(scheme: Scheme, key: String, label: String)] = [
        (.floatingSticks, "A", "A · Floating twin sticks"),
        (.fixedSticks, "B", "B · Fixed twin sticks"),
        (.autoAim, "C", "C · Move + auto-aim (flick to dodge)"),
        (.aimAssist, "D", "D · Twin sticks + aim assist"),
        (.fixedAutoAim, "E", "E · Fixed move stick + auto-aim (flick to dodge)"),
    ]
    /// Camera look options, with stable keys for persistence.
    static let looks: [(look: LookMode, key: String, label: String)] = [
        (.centered, "centered", "Centered"),
        (.aim, "aim", "Aim look · leads the aim while aiming (ETG)"),
        (.facing, "facing", "Facing look · always leads where you face"),
        (.enemy, "enemy", "Enemy look · leans toward the nearest enemy (auto-aim)"),
    ]
    /// Fire mode options, with stable keys for persistence.
    static let fireModes: [(mode: FireMode, key: String, label: String)] = [
        (.hold, "hold", "Hold · auto-fire while aiming"),
        (.tap, "tap", "Tap · one shot per touch"),
        (.release, "release", "Release · drag to aim, lift to fire"),
    ]
    private static let schemeKey = "controls.scheme"
    private static let fireModeKey = "controls.fireMode"
    private static let assistKey = "controls.assist"
    private static let cameraKey = "camera"
    private static let volumeKey = "audio.volume"

    var hud: HudData?
    var scheme: Scheme = GameModel.loadScheme() {
        didSet {
            UserDefaults.standard.set(Self.schemes.first { $0.scheme == scheme }?.key, forKey: Self.schemeKey)
            game?.setScheme(scheme: scheme)
        }
    }
    var fireMode: FireMode = GameModel.loadFireMode() {
        didSet {
            UserDefaults.standard.set(Self.fireModes.first { $0.mode == fireMode }?.key, forKey: Self.fireModeKey)
            game?.setFireMode(mode: fireMode)
        }
    }
    /// Only used by scheme D. The real default is a combat-tuning decision (issue #15).
    var assist: Float = UserDefaults.standard.object(forKey: GameModel.assistKey) as? Float ?? 0.5 {
        didSet {
            UserDefaults.standard.set(assist, forKey: Self.assistKey)
            game?.setAssistStrength(strength: assist)
        }
    }
    /// Camera look; applies live.
    var camera: CameraSettings = GameModel.loadCamera() {
        didSet {
            UserDefaults.standard.set([
                "look": Self.looks.first { $0.look == camera.look }?.key ?? "aim",
                "lead": Double(camera.lead),
                "smoothingSecs": Double(camera.smoothingSecs),
                "thumbClearance": Double(camera.thumbClearance),
            ], forKey: Self.cameraKey)
            game?.setCamera(settings: camera)
        }
    }
    /// Difficulty and tuning; the game applies them live.
    var runSettings: RunSettings = RunSettingsStore.load() {
        didSet {
            RunSettingsStore.save(runSettings)
            game?.setRunSettings(settings: runSettings)
        }
    }
    /// Plays Rust's per-frame sounds and music mood (GameUIView feeds it).
    @ObservationIgnored let audio = AudioEngine()
    /// Sound volumes, 0...1; apply live.
    var masterVolume: Float = GameModel.loadVolume("master", 1) { didSet { saveVolumes() } }
    var musicVolume: Float = GameModel.loadVolume("music", 0.6) { didSet { saveVolumes() } }
    var sfxVolume: Float = GameModel.loadVolume("sfx", 0.8) { didSet { saveVolumes() } }
    /// The game pauses while either holds; pause lives outside the sim (Rust stops stepping).
    var settingsOpen = false { didSet { syncPause() } }
    var appActive = true { didSet { syncPause() } }
    @ObservationIgnored private var game: Game?
    @ObservationIgnored private var paused = false

    init() {
        audio.setVolumes(master: masterVolume, music: musicVolume, sfx: sfxVolume)
    }

    private func saveVolumes() {
        UserDefaults.standard.set([
            "master": Double(masterVolume), "music": Double(musicVolume), "sfx": Double(sfxVolume),
        ], forKey: Self.volumeKey)
        audio.setVolumes(master: masterVolume, music: musicVolume, sfx: sfxVolume)
    }

    /// Pushes the persisted settings into a freshly created game.
    func attach(_ game: Game) {
        self.game = game
        game.setScheme(scheme: scheme)
        game.setFireMode(mode: fireMode)
        game.setAssistStrength(strength: assist)
        game.setCamera(settings: camera)
        if paused { game.pause() }
    }

    /// Only acts on transitions: `resume` resyncs the sim clock.
    private func syncPause() {
        let shouldPause = settingsOpen || !appActive
        guard shouldPause != paused else { return }
        paused = shouldPause
        if paused { game?.pause() } else { game?.resume() }
    }

    /// Tuning back to Rust's defaults; the difficulty stays.
    func resetTuning() {
        var settings = defaultRunSettings()
        settings.difficulty = runSettings.difficulty
        runSettings = settings
    }

    /// Restart (end-of-run overlay tap or the settings button); Rust turns it into the sim's RESTART input.
    func restart() {
        game?.restart()
    }

    private static func loadFireMode() -> FireMode {
        let key = UserDefaults.standard.string(forKey: fireModeKey)
        return fireModes.first { $0.key == key }?.mode ?? .hold
    }

    private static func loadScheme() -> Scheme {
        let key = UserDefaults.standard.string(forKey: schemeKey)
        return schemes.first { $0.key == key }?.scheme ?? .fixedAutoAim
    }

    private static func loadVolume(_ key: String, _ fallback: Float) -> Float {
        let saved = UserDefaults.standard.dictionary(forKey: volumeKey)?[key] as? Double
        return saved.map(Float.init) ?? fallback
    }

    private static func loadCamera() -> CameraSettings {
        var camera = defaultCameraSettings()
        guard let saved = UserDefaults.standard.dictionary(forKey: cameraKey) else { return camera }
        if let key = saved["look"] as? String, let found = looks.first(where: { $0.key == key }) {
            camera.look = found.look
        }
        if let value = saved["lead"] as? Double { camera.lead = Float(value) }
        if let value = saved["smoothingSecs"] as? Double { camera.smoothingSecs = Float(value) }
        if let value = saved["thumbClearance"] as? Double { camera.thumbClearance = Float(value) }
        return camera
    }
}

struct ContentView: View {
    @Bindable var model: GameModel

    var body: some View {
        ZStack {
            GameView(model: model).ignoresSafeArea()
            if let hud = model.hud {
                VStack(alignment: .leading, spacing: 6) {
                    Text("fps \(String(format: "%.1f", hud.fps))")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(.green)
                        .padding(6)
                        .background(.black.opacity(0.5))
                    HStack(spacing: 3) {
                        ForEach(0..<Int(hud.maxHp), id: \.self) { i in
                            Image(systemName: i < Int(hud.hp) ? "heart.fill" : "heart")
                        }
                    }
                    .font(.system(size: 16))
                    .foregroundStyle(.red)
                    // Phase pistol: a pip per charge; while venting, the refill's progress.
                    HStack(spacing: 3) {
                        ForEach(0..<Int(hud.maxCharges), id: \.self) { i in
                            Capsule()
                                .fill(i < Int(hud.charges) ? Color.cyan : Color.white.opacity(0.2))
                                .frame(width: 5, height: 12)
                        }
                        if hud.venting {
                            ProgressView(value: Double(hud.ventProgress))
                                .tint(.orange)
                                .frame(width: 44)
                            Text("VENT")
                                .font(.system(size: 10, weight: .bold, design: .monospaced))
                                .foregroundStyle(.orange)
                        }
                    }
                    // Room, and the wave while a fight is on.
                    Text(hud.wave > 0 ? "\(hud.room.capitalized) · wave \(hud.wave)/\(hud.waves)" : hud.room.capitalized)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(.white.opacity(0.85))
                        .padding(.horizontal, 6)
                        .padding(.vertical, 3)
                        .background(.black.opacity(0.5))
                }
                .allowsHitTesting(false)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .padding(.leading, 8)
                switch hud.run {
                case .dead(let canRestart):
                    EndOverlay(title: "You died", prompt: "Tap to restart", canRestart: canRestart) { model.restart() }
                case .won:
                    EndOverlay(title: "Derelict cleared", prompt: "Tap to start a new run", canRestart: true) { model.restart() }
                case .playing:
                    EmptyView()
                }
            }
            Button { model.settingsOpen = true } label: {
                Image(systemName: "gearshape.fill")
                    .font(.system(size: 18))
                    .padding(10)
                    .background(.white.opacity(0.15), in: Circle())
                    .foregroundStyle(.white)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
            .padding(8)
        }
        .background(.black)
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
        .sheet(isPresented: $model.settingsOpen) { ControlsSettings(model: model) }
    }
}

/// Covers the game once the run is over (died or won). Taps restart once allowed: after a
/// death, when the sim's death pause is over.
struct EndOverlay: View {
    let title: String
    let prompt: String
    let canRestart: Bool
    let restart: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            Text(title).font(.largeTitle.bold())
            Text(prompt).font(.title3).opacity(canRestart ? 1 : 0)
        }
        .foregroundStyle(.white)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.black.opacity(0.55))
        .contentShape(Rectangle())
        .onTapGesture { if canRestart { restart() } }
        .ignoresSafeArea()
    }
}

struct ControlsSettings: View {
    @Bindable var model: GameModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Picker("Scheme", selection: $model.scheme) {
                    ForEach(GameModel.schemes, id: \.key) { option in
                        Text(option.label).tag(option.scheme)
                    }
                }
                .pickerStyle(.inline)
                Section("Fire mode") {
                    Picker("Fire mode", selection: $model.fireMode) {
                        ForEach(GameModel.fireModes, id: \.key) { option in
                            Text(option.label).tag(option.mode)
                        }
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()
                }
                if model.scheme == .aimAssist {
                    Section("Aim assist strength") {
                        HStack {
                            Slider(value: $model.assist, in: 0...1)
                            Text(String(format: "%.2f", model.assist))
                                .font(.system(.body, design: .monospaced))
                        }
                    }
                }
                Section("Camera") {
                    Picker("Camera look", selection: $model.camera.look) {
                        ForEach(GameModel.looks, id: \.key) { option in
                            Text(option.label).tag(option.look)
                        }
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()
                    if model.camera.look != .centered {
                        SliderRow(label: "Lead", value: $model.camera.lead, range: 0...200, step: 5) {
                            "\(Int($0)) pt"
                        }
                        SliderRow(label: "Smoothing", value: $model.camera.smoothingSecs, range: 0...1, step: 0.05) {
                            String(format: "%.2f s", $0)
                        }
                    }
                    SliderRow(label: "Thumb clearance", value: $model.camera.thumbClearance, range: 0...120, step: 5) {
                        "\(Int($0)) pt"
                    }
                }
                Section("Sound") {
                    SliderRow(label: "Master", value: $model.masterVolume, range: 0...1, step: 0.05) {
                        "\(Int(($0 * 100).rounded()))%"
                    }
                    SliderRow(label: "Music", value: $model.musicVolume, range: 0...1, step: 0.05) {
                        "\(Int(($0 * 100).rounded()))%"
                    }
                    SliderRow(label: "Effects", value: $model.sfxVolume, range: 0...1, step: 0.05) {
                        "\(Int(($0 * 100).rounded()))%"
                    }
                }
                RunSettingsSections(model: model)
                Section {
                    // Pending restart survives the pause; it lands on the first tick after
                    // the sheet closes.
                    Button("Restart run", role: .destructive) {
                        model.restart()
                        dismiss()
                    }
                }
            }
            .navigationTitle("Settings")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}
