import CoreMotion
import UIKit

/// The phone's motion sensors, for tilt peek: gravity, polled once per frame.
final class GyroInput {
    private let motion = CMMotionManager()

    /// Runs the sensors only while tilt peek needs them.
    func run(_ active: Bool) {
        if active, motion.isDeviceMotionAvailable, !motion.isDeviceMotionActive {
            motion.deviceMotionUpdateInterval = 1.0 / 120
            motion.startDeviceMotionUpdates()
        } else if !active, motion.isDeviceMotionActive {
            motion.stopDeviceMotionUpdates()
        }
    }

    /// Tilt peek: gravity in screen axes (+x right, +y down), in g, for the interface
    /// orientation; `nil` with no motion data.
    func gravity(_ orientation: UIInterfaceOrientation) -> [Float]? {
        guard let g = motion.deviceMotion?.gravity else { return nil }
        // Device axes: +x right, +y up in portrait.
        switch orientation {
        case .landscapeRight: return [Float(-g.y), Float(-g.x)]
        case .landscapeLeft: return [Float(g.y), Float(g.x)]
        case .portraitUpsideDown: return [Float(-g.x), Float(g.y)]
        default: return [Float(g.x), Float(-g.y)]
        }
    }
}
