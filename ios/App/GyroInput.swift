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

    /// Tilt peek: the phone's tilt about the screen's two axes, in radians, for the
    /// interface orientation: [roll, rising as the right edge dips; pitch, rising as the
    /// bottom edge dips (toward upright)]. Angles, not gravity's components, so both axes
    /// respond evenly however upright the phone is held. `nil` with no motion data.
    func tilt(_ orientation: UIInterfaceOrientation) -> [Float]? {
        guard let g = motion.deviceMotion?.gravity else { return nil }
        // Gravity in screen axes (+x right, +y down); device axes are +x right, +y up in
        // portrait, +z out of the screen.
        let (x, y): (Double, Double)
        switch orientation {
        case .landscapeRight: (x, y) = (-g.y, -g.x)
        case .landscapeLeft: (x, y) = (g.y, g.x)
        case .portraitUpsideDown: (x, y) = (-g.x, g.y)
        default: (x, y) = (g.x, -g.y)
        }
        return [Float(asin(max(-1, min(1, x)))), Float(atan2(y, -g.z))]
    }
}
