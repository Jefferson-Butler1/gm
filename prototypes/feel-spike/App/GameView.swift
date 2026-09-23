import SwiftUI
import UIKit

struct GameView: UIViewRepresentable {
    let model: SpikeModel
    func makeUIView(context: Context) -> GameUIView { GameUIView(model: model) }
    func updateUIView(_ view: GameUIView, context: Context) {}
}

/// Layer-backed view. Swift owns the display link and touches; Rust owns sim + rendering.
final class GameUIView: UIView {
    override class var layerClass: AnyClass { CAMetalLayer.self }

    private let model: SpikeModel
    private var game: Game?
    private var link: CADisplayLink?
    private var lastSeq: UInt64 = 0
    // Environment diagnostics, logged ~1/s alongside Rust's stats line.
    private var lastEnvLog: CFTimeInterval = 0
    private var linkIntervalSum: Double = 0
    private var linkIntervalCount = 0

    init(model: SpikeModel) {
        self.model = model
        super.init(frame: .zero)
        isMultipleTouchEnabled = true
        NotificationCenter.default.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { [weak self] _ in
            self?.link?.isPaused = true
        }
        NotificationCenter.default.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
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

    override func layoutSubviews() {
        super.layoutSubviews()
        let s = layer.contentsScale
        let wPx = UInt32(bounds.width * s), hPx = UInt32(bounds.height * s)
        guard wPx > 0, hPx > 0 else { return }
        if let game {
            game.resize(widthPx: wPx, heightPx: hPx, widthPt: Float(bounds.width), heightPt: Float(bounds.height))
            return
        }
        // Rust holds a raw pointer to the layer; this view keeps it alive for the Game's lifetime.
        let ptr = UInt64(UInt(bitPattern: Unmanaged.passUnretained(layer).toOpaque()))
        let g = Game(layerPtr: ptr, widthPx: wPx, heightPx: hPx, widthPt: Float(bounds.width), heightPt: Float(bounds.height))
        game = g
        model.game = g
        // SPIKE_STRESS=<n> overrides the initial stress level (launch with DEVICECTL_CHILD_SPIKE_STRESS=500).
        if let env = ProcessInfo.processInfo.environment["SPIKE_STRESS"], let n = UInt32(env) {
            g.setStress(count: n)
        } else {
            g.setStress(count: SpikeModel.stressLevels[model.stressIndex])
        }
        // SPIKE_SCHEME=A|B|C|D picks the starting control scheme (DEVICECTL_CHILD_SPIKE_SCHEME=C).
        if let env = ProcessInfo.processInfo.environment["SPIKE_SCHEME"],
           let i = ["A", "B", "C", "D"].firstIndex(of: env.uppercased()) {
            model.schemeIndex = i
        }
        g.setAssist(strength: SpikeModel.assistLevels[model.assistIndex])
        g.setScheme(scheme: SpikeModel.schemes[model.schemeIndex].0)

        let link = CADisplayLink(target: self, selector: #selector(step))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 80, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        self.link = link
        print("[spike] display link started, maxFPS=\(window?.windowScene?.screen.maximumFramesPerSecond ?? -1)")
    }

    @objc private func step(_ link: CADisplayLink) {
        guard let game else { return }
        linkIntervalSum += link.targetTimestamp - link.timestamp
        linkIntervalCount += 1
        if link.timestamp - lastEnvLog >= 1 {
            logEnv(link)
        }
        let hud = game.frame(timestamp: link.timestamp, targetTimestamp: link.targetTimestamp)
        // Rust refreshes stats ~4x/sec; only touch SwiftUI state when they change.
        if hud.seq != lastSeq {
            lastSeq = hud.seq
            model.hud = hud
        }
    }

    private func logEnv(_ link: CADisplayLink) {
        lastEnvLog = link.timestamp
        let pi = ProcessInfo.processInfo
        let thermal = ["nominal", "fair", "serious", "critical"][pi.thermalState.rawValue]
        let metal = layer as! CAMetalLayer
        let avgInterval = linkIntervalSum / Double(max(linkIntervalCount, 1)) * 1e3
        linkIntervalSum = 0
        linkIntervalCount = 0
        // displaySyncEnabled is macOS-only; on iOS presentation is always vsynced.
        fputs("[spike-env] thermal=\(thermal) lowPower=\(pi.isLowPowerModeEnabled) link_interval_ms=\(String(format: "%.2f", avgInterval)) maxFPS=\(window?.windowScene?.screen.maximumFramesPerSecond ?? -1) maxDrawables=\(metal.maximumDrawableCount) drawable=\(Int(metal.drawableSize.width))x\(Int(metal.drawableSize.height)) presentsWithTransaction=\(metal.presentsWithTransaction)\n", stderr)
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
