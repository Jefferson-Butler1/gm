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
    var hud: HudData?
}

struct ContentView: View {
    let model: GameModel

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
        }
        .background(.black)
        .statusBarHidden(true)
        .persistentSystemOverlays(.hidden)
    }
}
