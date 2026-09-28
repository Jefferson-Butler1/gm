import AVFoundation
import MediaPlayer
import UIKit

/// The volume buttons as game buttons (fire mode "Volume"): each up press is one shot,
/// each down press a dodge. iOS has no API for them, so this watches the system output
/// volume and puts it back after each press; a hidden `MPVolumeView` keeps the volume
/// HUD away. iOS reports presses, not holds; a held button repeats on its own.
///
/// While active the system volume is pinned mid-way; use the in-game volume.
final class VolumeButtons {
    var onShot: () -> Void = {}
    var onDodge: () -> Void = {}

    private static let rest: Float = 0.5

    private let volumeView = MPVolumeView(frame: CGRect(x: -2000, y: -2000, width: 1, height: 1))
    private var observation: NSKeyValueObservation?

    func start(in window: UIWindow) {
        guard observation == nil else { return }
        window.addSubview(volumeView)
        try? AVAudioSession.sharedInstance().setActive(true)
        setVolume(Self.rest)
        observation = AVAudioSession.sharedInstance().observe(\.outputVolume, options: [.old, .new]) { [weak self] _, change in
            guard let self, let new = change.newValue, let old = change.oldValue else { return }
            DispatchQueue.main.async { self.pressed(up: new > old, volume: new) }
        }
    }

    func stop() {
        observation = nil
        volumeView.removeFromSuperview()
    }

    private func pressed(up: Bool, volume: Float) {
        // Our own reset back to rest.
        if abs(volume - Self.rest) < 0.001 { return }
        if up { onShot() } else { onDodge() }
        setVolume(Self.rest)
    }

    private func setVolume(_ value: Float) {
        let slider = volumeView.subviews.compactMap { $0 as? UISlider }.first
        DispatchQueue.main.async { slider?.value = value }
    }
}
