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
    @ObservationIgnored private var game: Game?

    /// Pushes the persisted settings into a freshly created game.
    func attach(_ game: Game) {
        self.game = game
        game.setScheme(scheme: scheme)
        game.setAssistStrength(strength: assist)
    }

    private static func loadScheme() -> Scheme {
        let key = UserDefaults.standard.string(forKey: schemeKey)
        return schemes.first { $0.key == key }?.scheme ?? .fixedSticks
    }
}

struct ContentView: View {
    @Bindable var model: GameModel
    @State private var showingControls = false

    var body: some View {
        ZStack {
            GameView(model: model).ignoresSafeArea()
            if let hud = model.hud {
                Text("fps \(String(format: "%.1f", hud.fps))")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(.green)
                    .padding(6)
                    .background(.black.opacity(0.5))
                    .allowsHitTesting(false)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                    .padding(.leading, 8)
            }
            Button { showingControls = true } label: {
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
        .sheet(isPresented: $showingControls) { ControlsSettings(model: model) }
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
