import GameController

/// The current game controller as the game's input, polled once per frame: left stick
/// moves, right stick aims, right trigger fires, left trigger or B (circle) dodges, X
/// (square) vents, Y (triangle) or the right shoulder uses an EMP. Menu opens and closes
/// settings.
final class GamepadInput {
    var onMenu: () -> Void = {}

    private var observer: NSObjectProtocol?

    init() {
        observer = NotificationCenter.default.addObserver(forName: .GCControllerDidBecomeCurrent, object: nil, queue: .main) { [weak self] note in
            self?.watchMenu(note.object as? GCController)
        }
        watchMenu(GCController.current)
    }

    deinit {
        if let observer { NotificationCenter.default.removeObserver(observer) }
    }

    /// `nil` without an extended gamepad. Sticks flip to view axes (+y down).
    func poll() -> GamepadState? {
        guard let pad = GCController.current?.extendedGamepad else { return nil }
        return GamepadState(
            moveX: pad.leftThumbstick.xAxis.value, moveY: -pad.leftThumbstick.yAxis.value,
            aimX: pad.rightThumbstick.xAxis.value, aimY: -pad.rightThumbstick.yAxis.value,
            fire: pad.rightTrigger.isPressed,
            dodge: pad.leftTrigger.isPressed || pad.buttonB.isPressed,
            vent: pad.buttonX.isPressed,
            emp: pad.buttonY.isPressed || pad.rightShoulder.isPressed)
    }

    private func watchMenu(_ controller: GCController?) {
        controller?.extendedGamepad?.buttonMenu.pressedChangedHandler = { [weak self] _, _, pressed in
            if pressed { self?.onMenu() }
        }
    }
}
