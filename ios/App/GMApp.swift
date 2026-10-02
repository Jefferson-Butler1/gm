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
    /// The Controls options: each a touch scheme with the fire mode it plays best with,
    /// with a stable key for persistence. The first is the default.
    static let controls: [(key: String, label: String, scheme: Scheme, fireMode: FireMode)] = [
        ("brawlStars", "Brawl Stars / ETG mobile · tap for a quick auto shot, drag to aim, release to fire", .fireButton, .release),
        ("soulKnight", "Soul Knight · fire button: hold to auto-aim, drag to aim", .fireButton, .hold),
        ("twinStick", "Twin-stick (ETG, Nuclear Throne) · aiming fires", .fixedSticks, .hold),
        ("tapToFire", "Tap to fire · auto-aim", .fixedAutoAim, .tap),
        ("claw", "Claw (PUBG, CoD Mobile) · thumbs move and aim, index fingers fire and dodge", .claw, .hold),
    ]
    /// Camera look options, with stable keys for persistence.
    static let looks: [(look: LookMode, key: String, label: String)] = [
        (.centered, "centered", "Centered"),
        (.aim, "aim", "Aim look · leads the aim while aiming (ETG)"),
        (.facing, "facing", "Facing look · always leads where you face"),
        (.enemy, "enemy", "Enemy look · leans toward the nearest enemy (auto-aim)"),
    ]
    private static let controlsKey = "controls"
    private static let assistKey = "controls.assist"
    private static let cameraKey = "camera"
    private static let volumeKey = "audio.volume.v2"  // v2: sound starts off
    private static let hapticsKey = "haptics"

    var hud: HudData?
    var controlsKey: String = UserDefaults.standard.string(forKey: GameModel.controlsKey)
        .flatMap { key in GameModel.controls.first { $0.key == key }?.key } ?? GameModel.controls[0].key {
        didSet {
            UserDefaults.standard.set(controlsKey, forKey: Self.controlsKey)
            game?.setScheme(scheme: scheme)
            game?.setFireMode(mode: fireMode)
            game?.setLayout(layout: layouts[controlsKey])
        }
    }
    /// Control layouts from the editor, by preset key; a preset without one uses Rust's
    /// default. Applied through `setLayout` on attach, preset change and editor save.
    var layouts: [String: ControlLayout] = ControlLayoutStore.load() {
        didSet { ControlLayoutStore.save(layouts) }
    }
    /// The layout editor is open over the paused game.
    var editingLayout = false { didSet { syncPause() } }
    /// The viewport GameUIView last laid out: what the layout editor places controls on.
    var viewport: Viewport?
    var scheme: Scheme { (Self.controls.first { $0.key == controlsKey } ?? Self.controls[0]).scheme }
    var fireMode: FireMode { (Self.controls.first { $0.key == controlsKey } ?? Self.controls[0]).fireMode }
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
                "tiltPeek": Double(camera.tiltPeek),
            ], forKey: Self.cameraKey)
            game?.setCamera(settings: camera)
        }
    }
    /// Swift-only: GameUIView skips the frame's haptics when off.
    var haptics: Bool = UserDefaults.standard.object(forKey: GameModel.hapticsKey) as? Bool ?? true {
        didSet { UserDefaults.standard.set(haptics, forKey: Self.hapticsKey) }
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
    var masterVolume: Float = GameModel.loadVolume("master", 0) { didSet { saveVolumes() } }
    var musicVolume: Float = GameModel.loadVolume("music", 0.6) { didSet { saveVolumes() } }
    var sfxVolume: Float = GameModel.loadVolume("sfx", 0.8) { didSet { saveVolumes() } }
    /// The game pauses while either holds, or the layout editor is open; pause lives outside
    /// the sim (Rust stops stepping).
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
        game.setLayout(layout: layouts[controlsKey])
        game.setAssistStrength(strength: assist)
        game.setCamera(settings: camera)
        if paused { game.pause() }
    }

    /// Only acts on transitions: `resume` resyncs the sim clock.
    private func syncPause() {
        let shouldPause = settingsOpen || editingLayout || !appActive
        guard shouldPause != paused else { return }
        paused = shouldPause
        if paused { game?.pause() } else { game?.resume() }
    }

    /// Shows `layout` live while the editor drags, without saving it.
    func previewLayout(_ layout: ControlLayout) {
        game?.setLayout(layout: layout)
    }

    /// Closes the layout editor. Saving stores `layout` for the current preset (the
    /// default is stored as no layout) and exports every preset's layout for reading off
    /// the device; cancelling (`nil`) puts the saved one back.
    func finishLayoutEditing(saving layout: ControlLayout?) {
        if let layout, let viewport {
            let isDefault = layout == defaultControlLayout(scheme: scheme, viewport: viewport)
            layouts[controlsKey] = isDefault ? nil : layout
            ControlLayoutStore.export(layouts, active: controlsKey, viewport: viewport)
        }
        game?.setLayout(layout: layouts[controlsKey])
        editingLayout = false
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

    private static func loadCamera() -> CameraSettings {
        var camera = defaultCameraSettings()
        guard let saved = UserDefaults.standard.dictionary(forKey: cameraKey) else { return camera }
        if let key = saved["look"] as? String, let found = looks.first(where: { $0.key == key }) {
            camera.look = found.look
        }
        if let value = saved["lead"] as? Double { camera.lead = Float(value) }
        if let value = saved["smoothingSecs"] as? Double { camera.smoothingSecs = Float(value) }
        if let value = saved["thumbClearance"] as? Double { camera.thumbClearance = Float(value) }
        if let value = saved["tiltPeek"] as? Double { camera.tiltPeek = Float(value) }
        return camera
    }

    private static func loadVolume(_ key: String, _ fallback: Float) -> Float {
        let saved = UserDefaults.standard.dictionary(forKey: volumeKey)?[key] as? Double
        return saved.map(Float.init) ?? fallback
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
                    CreditsRoll { model.restart() }
                case .playing:
                    EmptyView()
                }
            }
            if model.editingLayout, let viewport = model.viewport {
                LayoutEditor(model: model, viewport: viewport)
            } else {
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
                Section("Controls") {
                    Picker("Controls", selection: $model.controlsKey) {
                        ForEach(GameModel.controls, id: \.key) { option in
                            Text(option.label).tag(option.key)
                        }
                    }
                    .pickerStyle(.inline)
                    .labelsHidden()
                    // Pause holds across the handoff: the editor opens before the sheet closes.
                    Button("Edit layout") {
                        model.editingLayout = true
                        dismiss()
                    }
                    .disabled(model.viewport == nil)
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
                Section("Feedback") {
                    Toggle("Haptics", isOn: $model.haptics)
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
                    SliderRow(label: "Tilt peek (gyro)", value: $model.camera.tiltPeek, range: 0...800, step: 50) {
                        $0 == 0 ? "off" : "\(Int($0)) pt/g"
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
