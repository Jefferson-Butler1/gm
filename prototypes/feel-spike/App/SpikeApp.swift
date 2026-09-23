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

    var hud = HudSnapshot()
    var stressIndex = 0
    @ObservationIgnored var game: Game?

    func cycleStress() {
        stressIndex = (stressIndex + 1) % Self.stressLevels.count
        game?.setStress(count: Self.stressLevels[stressIndex])
    }
}

extension HudSnapshot {
    init() {
        self.init(seq: 0, framesTotal: 0, displayFps: 0, frameMsAvg: 0, frameMsP99: 0, frameMsMax: 0,
                  rustMsAvg: 0, rustMsP99: 0, acquireMsAvg: 0, simStepMsAvg: 0, simStepsPerSec: 0,
                  sprites: 0, bullets: 0, drawableW: 0, drawableH: 0)
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
            Button {
                model.cycleStress()
            } label: {
                Text("load: \(SpikeModel.stressLevels[model.stressIndex])")
                    .font(.system(size: 14, weight: .semibold, design: .monospaced))
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .background(.white.opacity(0.15), in: Capsule())
                    .foregroundStyle(.white)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topTrailing)
            .padding(8)
        }
        .background(.black)
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
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
        }
        .font(.system(size: 11, design: .monospaced))
        .foregroundStyle(.green)
        .padding(6)
        .background(.black.opacity(0.5))
    }
}
