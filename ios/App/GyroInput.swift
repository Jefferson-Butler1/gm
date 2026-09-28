import CoreMotion
import UIKit

/// Gyro aim (prototype): the phone's turn rate about the vertical, polled once per frame.
/// Projecting the rotation rate onto gravity makes it the same "turn left/right" whether the
/// phone is held upright or tilted flat, in either landscape orientation.
final class GyroInput {
    private let motion = CMMotionManager()

    /// Runs the sensors only while gyro aim or tilt peek needs them.
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

    /// Radians per second, clockwise on screen (the aim angle's direction); 0 with no motion
    /// data.
    func rate() -> Float {
        guard let data = motion.deviceMotion else { return 0 }
        let (r, g) = (data.rotationRate, data.gravity)
        let length = (g.x * g.x + g.y * g.y + g.z * g.z).squareRoot()
        guard length > 0.1 else { return 0 }
        // Up is -gravity; a counterclockwise turn about up (right-handed) turns the aim
        // counterclockwise on screen, which is negative for the aim angle.
        let aboutUp = -(r.x * g.x + r.y * g.y + r.z * g.z) / length
        return Float(-aboutUp)
    }
}
