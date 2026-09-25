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
    ]
    private static let schemeKey = "controls.scheme"
    private static let assistKey = "controls.assist"

    var hud: HudData?
    var scheme: Scheme = GameModel.loadScheme() {
        didSet {
            UserDefaults.standard.set(Self.schemes.first { $0.scheme == scheme }?.key, forKey: Self.schemeKey)
            game?.setScheme(scheme: scheme)
        }
    }
    /// Only used by scheme D. The real default is a combat-tuning decision (issue #15).
    var assist: Float = UserDefaults.standard.object(forKey: GameModel.assistKey) as? Float ?? 0.5 {
        didSet {
            UserDefaults.standard.set(assist, forKey: Self.assistKey)
            game?.setAssistStrength(strength: assist)
        }
    }
    /// The game pauses while either holds; pause lives outside the sim (Rust stops stepping).
    var settingsOpen = false { didSet { syncPause() } }
    var appActive = true { didSet { syncPause() } }
    @ObservationIgnored private var game: Game?
    @ObservationIgnored private var paused = false

    /// Pushes the persisted settings into a freshly created game.
    func attach(_ game: Game) {
        self.game = game
        game.setScheme(scheme: scheme)
        game.setAssistStrength(strength: assist)
        if paused { game.pause() }
    }

    /// Only acts on transitions: `resume` resyncs the sim clock.
    private func syncPause() {
        let shouldPause = settingsOpen || !appActive
        guard shouldPause != paused else { return }
        paused = shouldPause
        if paused { game?.pause() } else { game?.resume() }
    }

    /// Tap-to-restart; Rust turns it into the sim's RESTART input.
    func restart() {
        game?.restart()
    }

    private static func loadScheme() -> Scheme {
        let key = UserDefaults.standard.string(forKey: schemeKey)
        return schemes.first { $0.key == key }?.scheme ?? .fixedSticks
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
                }
                .allowsHitTesting(false)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .padding(.leading, 8)
                if case .dead(let canRestart) = hud.run {
                    DeadOverlay(canRestart: canRestart) { model.restart() }
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

/// Covers the game while dead. Taps restart once the sim's death pause is over.
struct DeadOverlay: View {
    let canRestart: Bool
    let restart: () -> Void

    var body: some View {
        VStack(spacing: 12) {
            Text("You died").font(.largeTitle.bold())
            Text("Tap to restart").font(.title3).opacity(canRestart ? 1 : 0)
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
                if model.scheme == .aimAssist {
                    Section("Aim assist strength") {
                        HStack {
                            Slider(value: $model.assist, in: 0...1)
                            Text(String(format: "%.2f", model.assist))
                                .font(.system(.body, design: .monospaced))
                        }
                    }
                }
            }
            .navigationTitle("Controls")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}
