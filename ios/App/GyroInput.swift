import CoreMotion

/// Gyro aim (prototype): the phone's turn rate about the vertical, polled once per frame.
/// Projecting the rotation rate onto gravity makes it the same "turn left/right" whether the
/// phone is held upright or tilted flat, in either landscape orientation.
final class GyroInput {
    private let motion = CMMotionManager()

    /// Radians per second, clockwise on screen (the aim angle's direction); 0 while `enabled`
    /// is off, which also stops the sensors.
    func poll(enabled: Bool) -> Float {
        guard enabled, motion.isDeviceMotionAvailable else {
            if motion.isDeviceMotionActive { motion.stopDeviceMotionUpdates() }
            return 0
        }
        if !motion.isDeviceMotionActive {
            motion.deviceMotionUpdateInterval = 1.0 / 120
            motion.startDeviceMotionUpdates()
        }
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
