import SwiftUI

@main
struct SpikeApp: App {
    @State private var model = SpikeModel()

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
        }
    }
}

@Observable
final class SpikeModel {
    static let stressLevels: [UInt32] = [0, 500, 2000]
    static let schemes: [(Scheme, String)] = [
        (.floatingSticks, "A floating"), (.fixedSticks, "B fixed"), (.autoAim, "C auto-aim"), (.aimAssist, "D assist"),
    ]
    static let assistLevels: [Float] = [0, 0.5, 1.0]

    var hud = HudSnapshot()
    var stressIndex = 0
    var schemeIndex = 0
    var assistIndex = 1
    @ObservationIgnored var game: Game?

    func cycleStress() {
        stressIndex = (stressIndex + 1) % Self.stressLevels.count
        game?.setStress(count: Self.stressLevels[stressIndex])
    }

    func cycleScheme() {
        schemeIndex = (schemeIndex + 1) % Self.schemes.count
        game?.setScheme(scheme: Self.schemes[schemeIndex].0)
    }

    func cycleAssist() {
        assistIndex = (assistIndex + 1) % Self.assistLevels.count
        game?.setAssist(strength: Self.assistLevels[assistIndex])
    }
}

extension HudSnapshot {
    init() {
        self.init(seq: 0, framesTotal: 0, displayFps: 0, frameMsAvg: 0, frameMsP99: 0, frameMsMax: 0,
                  rustMsAvg: 0, rustMsP99: 0, acquireMsAvg: 0, simStepMsAvg: 0, simStepsPerSec: 0,
                  sprites: 0, bullets: 0, hits: 0, shots: 0, drawableW: 0, drawableH: 0)
    }
}

struct ContentView: View {
    let model: SpikeModel

    var body: some View {
        ZStack {
            GameView(model: model).ignoresSafeArea()
            HUDView(hud: model.hud)
                .allowsHitTesting(false)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .padding(.leading, 8)
            VStack(alignment: .trailing, spacing: 6) {
                pill("scheme: \(SpikeModel.schemes[model.schemeIndex].1)") { model.cycleScheme() }
                if SpikeModel.schemes[model.schemeIndex].0 == .aimAssist {
                    pill("assist: \(SpikeModel.assistLevels[model.assistIndex])") { model.cycleAssist() }
                }
                pill("load: \(SpikeModel.stressLevels[model.stressIndex])") { model.cycleStress() }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
            .padding(8)
        }
        .background(.black)
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
    }

    private func pill(_ title: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.system(size: 14, weight: .semibold, design: .monospaced))
                .padding(.horizontal, 12).padding(.vertical, 8)
                .background(.white.opacity(0.15), in: Capsule())
                .foregroundStyle(.white)
        }
    }
}

struct HUDView: View {
    let hud: HudSnapshot

    var body: some View {
        let f = { (v: Double) in String(format: "%.2f", v) }
        VStack(alignment: .leading, spacing: 1) {
            Text("fps \(String(format: "%.1f", hud.displayFps))  frames \(hud.framesTotal)")
            Text("frame ms avg \(f(hud.frameMsAvg)) p99 \(f(hud.frameMsP99)) max \(f(hud.frameMsMax))")
            Text("rust frame() ms avg \(f(hud.rustMsAvg)) p99 \(f(hud.rustMsP99))")
            Text("  acquire ms avg \(f(hud.acquireMsAvg))")
            Text("sim step ms \(String(format: "%.4f", hud.simStepMsAvg))  steps/s \(hud.simStepsPerSec)")
            Text("sprites \(hud.sprites) bullets \(hud.bullets)  \(hud.drawableW)x\(hud.drawableH)")
            Text("hits \(hud.hits) / shots \(hud.shots)  acc \(hud.shots == 0 ? 0 : Int(Double(hud.hits) / Double(hud.shots) * 100))%")
                .font(.system(size: 15, weight: .bold, design: .monospaced))
                .foregroundStyle(.yellow)
        }
        .font(.system(size: 11, design: .monospaced))
        .foregroundStyle(.green)
        .padding(6)
        .background(.black.opacity(0.5))
    }
}
