import SwiftUI
import UIKit

struct GameView: UIViewRepresentable {
    let model: GameModel
    func makeUIView(context: Context) -> GameUIView { GameUIView(model: model) }
    func updateUIView(_ view: GameUIView, context: Context) {}
}

/// Layer-backed view. Swift owns the lifecycle, display link and touches; Rust owns the
/// sim and rendering.
final class GameUIView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    private let model: GameModel
    private var game: Game?
    private var link: CADisplayLink?
    private var lastSeq: UInt64 = 0

    init(model: GameModel) {
        self.model = model
        super.init(frame: .zero)
        isMultipleTouchEnabled = true
        NotificationCenter.default.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            self?.link?.isPaused = true
            self?.model.appActive = false
        }
        NotificationCenter.default.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
            self?.model.appActive = true
            self?.link?.isPaused = false
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if let screen = window?.windowScene?.screen {
            contentScaleFactor = screen.nativeScale
            layer.contentsScale = screen.nativeScale
        }
    }

    override func safeAreaInsetsDidChange() {
        super.safeAreaInsetsDidChange()
        setNeedsLayout()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let s = layer.contentsScale
        // The view ignores the safe area (full-bleed rendering), so read the insets from the
        // window; Rust keeps the dodge button inside them.
        let inset = window?.safeAreaInsets ?? .zero
        let viewport = Viewport(
            pixelWidth: UInt32(bounds.width * s), pixelHeight: UInt32(bounds.height * s),
            pointWidth: Float(bounds.width), pointHeight: Float(bounds.height),
            safeTop: Float(inset.top), safeLeft: Float(inset.left),
            safeBottom: Float(inset.bottom), safeRight: Float(inset.right))
        guard viewport.pixelWidth > 0, viewport.pixelHeight > 0 else { return }
        if let game {
            game.resize(viewport: viewport)
            return
        }
        // Rust holds a raw pointer to the layer; this view keeps it alive for the Game's lifetime.
        let ptr = UInt64(UInt(bitPattern: Unmanaged.passUnretained(layer).toOpaque()))
        do {
            let game = try Game(layerPtr: ptr, viewport: viewport, seed: UInt64.random(in: .min ... .max),
                                settings: model.runSettings)
            model.attach(game)
            self.game = game
        } catch {
            fputs("[gm] Game init failed: \(error)\n", stderr)
            return
        }

        let link = CADisplayLink(target: self, selector: #selector(step))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        self.link = link
        // Low Power Mode and thermal throttling cap the display at 60 Hz; log them to explain fps.
        let pi = ProcessInfo.processInfo
        fputs("[gm] display link started, maxFPS=\(window?.windowScene?.screen.maximumFramesPerSecond ?? -1) lowPower=\(pi.isLowPowerModeEnabled) thermal=\(pi.thermalState.rawValue)\n", stderr)
    }

    @objc private func step(_ link: CADisplayLink) {
        guard let game else { return }
        let hud = game.frame(timestamp: link.timestamp, targetTimestamp: link.targetTimestamp)
        // Sounds and mood are per frame, not gated on `seq`.
        model.audio.frame(sounds: hud.sounds, mood: hud.mood)
        // Only touch SwiftUI state when Rust publishes new numbers.
        if hud.seq != lastSeq {
            lastSeq = hud.seq
            model.hud = hud
        }
    }

    private func forward(_ touches: Set<UITouch>, _ phase: TouchPhase) {
        guard let game else { return }
        for t in touches {
            let p = t.location(in: self)
            let id = UInt64(UInt(bitPattern: ObjectIdentifier(t).hashValue))
            game.touch(id: id, phase: phase, x: Float(p.x), y: Float(p.y))
        }
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) { forward(touches, .began) }
    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) { forward(touches, .moved) }
    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) { forward(touches, .ended) }
    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { forward(touches, .ended) }
}
