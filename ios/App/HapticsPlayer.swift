import UIKit

/// Plays the haptics Rust picks each frame (`HudData.haptics`). Impacts for combat beats,
/// notification patterns for the run's big moments.
final class HapticsPlayer {
    private let light = UIImpactFeedbackGenerator(style: .light)
    private let medium = UIImpactFeedbackGenerator(style: .medium)
    private let heavy = UIImpactFeedbackGenerator(style: .heavy)
    private let soft = UIImpactFeedbackGenerator(style: .soft)
    private let rigid = UIImpactFeedbackGenerator(style: .rigid)
    private let notification = UINotificationFeedbackGenerator()

    init() {
        // Wakes the Taptic Engine so the first haptic isn't late.
        [light, medium, heavy, soft, rigid].forEach { $0.prepare() }
        notification.prepare()
    }

    func play(_ haptics: [Haptic]) {
        for haptic in haptics {
            switch haptic {
            case .shot: light.impactOccurred(intensity: 0.35)
            case .enemyHit: light.impactOccurred(intensity: 0.8)
            case .roll: soft.impactOccurred(intensity: 0.7)
            case .ventStart: soft.impactOccurred(intensity: 1)
            case .ventDone: rigid.impactOccurred(intensity: 0.6)
            case .enemyKilled: medium.impactOccurred()
            case .waveStart: notification.notificationOccurred(.warning)
            case .hatchSeal: rigid.impactOccurred(intensity: 1)
            case .emp: heavy.impactOccurred(intensity: 0.9)
            case .playerHurt: heavy.impactOccurred()
            case .won: notification.notificationOccurred(.success)
            case .playerDied: notification.notificationOccurred(.error)
            }
        }
    }
}
