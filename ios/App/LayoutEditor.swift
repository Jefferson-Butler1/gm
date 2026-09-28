import SwiftUI

extension ControlKind {
    static let all: [ControlKind] = [.moveStick, .aimStick, .dodge, .vent, .fire]

    /// Stable key for persistence and the exported JSON.
    var key: String {
        switch self {
        case .moveStick: "move"
        case .aimStick: "aim"
        case .dodge: "dodge"
        case .vent: "vent"
        case .fire: "fire"
        }
    }

    var path: WritableKeyPath<ControlLayout, ControlPlacement> {
        switch self {
        case .moveStick: \.moveStick
        case .aimStick: \.aimStick
        case .dodge: \.dodge
        case .vent: \.vent
        case .fire: \.fire
        }
    }

    func label(_ scheme: Scheme) -> String {
        switch self {
        case .moveStick: "Move"
        case .aimStick: scheme == .fireButton ? "Aim/Fire" : "Aim"
        case .dodge: "Dodge"
        case .vent: "Vent"
        case .fire: "Fire"
        }
    }

    /// Matches the in-game button colors (render's DODGE/VENT/FIRE_COLOR).
    var color: Color {
        switch self {
        case .moveStick, .aimStick: .white
        case .dodge: Color(red: 0.3, green: 0.9, blue: 1)
        case .vent: Color(red: 1, green: 0.75, blue: 0.25)
        case .fire: Color(red: 1, green: 0.35, blue: 0.3)
        }
    }
}

/// Persists the per-preset control layouts in UserDefaults, and exports them on save to
/// `Documents/control-layouts.json` for reading off the device:
/// `xcrun devicectl device copy from --device <id> --domain-type appDataContainer
/// --domain-identifier com.jeffersonbutler.gm --source Documents/control-layouts.json --destination .`
enum ControlLayoutStore {
    private static let key = "controls.layouts"
    private static let exportName = "control-layouts.json"

    /// Saved as `[preset: [control: [x, y, size]]]`; a preset missing a control is dropped.
    static func load() -> [String: ControlLayout] {
        guard let saved = UserDefaults.standard.dictionary(forKey: key) as? [String: [String: [Double]]] else {
            return [:]
        }
        return saved.compactMapValues { controls in
            func place(_ kind: ControlKind) -> ControlPlacement? {
                guard let v = controls[kind.key], v.count == 3 else { return nil }
                return ControlPlacement(x: Float(v[0]), y: Float(v[1]), size: Float(v[2]))
            }
            guard let move = place(.moveStick), let aim = place(.aimStick), let dodge = place(.dodge),
                  let vent = place(.vent), let fire = place(.fire) else { return nil }
            return ControlLayout(moveStick: move, aimStick: aim, dodge: dodge, vent: vent, fire: fire)
        }
    }

    static func save(_ layouts: [String: ControlLayout]) {
        let saved = layouts.mapValues { layout in
            Dictionary(uniqueKeysWithValues: ControlKind.all.map { kind in
                let p = layout[keyPath: kind.path]
                return (kind.key, [Double(p.x), Double(p.y), Double(p.size)])
            })
        }
        UserDefaults.standard.set(saved, forKey: key)
    }

    /// Every preset's layout (saved or default) with the screen it was saved on, and each
    /// shown control resolved to points on that screen. Logs a one-line summary of `active`.
    static func export(_ layouts: [String: ControlLayout], active: String, viewport: Viewport) {
        func round(_ value: Float, _ places: Double) -> Double {
            let scale = pow(10, places)
            return (Double(value) * scale).rounded() / scale
        }
        var summary = ""
        var presets: [String: Any] = [:]
        for preset in GameModel.controls {
            let layout = layouts[preset.key] ?? defaultControlLayout(scheme: preset.scheme, viewport: viewport)
            var controls: [String: Any] = [:]
            for placed in placeControls(scheme: preset.scheme, layout: layout, viewport: viewport) {
                let p = layout[keyPath: placed.kind.path]
                controls[placed.kind.key] = [
                    "label": placed.kind.label(preset.scheme),
                    "x": round(p.x, 4), "y": round(p.y, 4), "size": round(p.size, 3),
                    "centerPt": ["x": round(placed.x, 1), "y": round(placed.y, 1)],
                    "radiusPt": round(placed.radius, 1),
                    "hitRadiusPt": round(placed.hitRadius, 1),
                ]
                if preset.key == active {
                    summary += String(format: " %@ (%.2f,%.2f)x%.2f", placed.kind.key, p.x, p.y, p.size)
                }
            }
            presets[preset.key] = [
                "label": preset.label,
                "scheme": "\(preset.scheme)",
                "custom": layouts[preset.key] != nil,
                "controls": controls,
            ]
        }
        let json: [String: Any] = [
            "savedAt": ISO8601DateFormatter().string(from: Date()),
            "activePreset": active,
            "units": "x, y: control center as a fraction of the safe area (0 = left/top edge, 1 = right/bottom). "
                + "size: scale on the default radius. *Pt: resolved in points on this screen, origin top-left.",
            "screenPt": ["width": round(viewport.pointWidth, 1), "height": round(viewport.pointHeight, 1)],
            "safeAreaInsetsPt": [
                "top": round(viewport.safeTop, 1), "left": round(viewport.safeLeft, 1),
                "bottom": round(viewport.safeBottom, 1), "right": round(viewport.safeRight, 1),
            ],
            "presets": presets,
        ]
        do {
            let docs = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask,
                                                   appropriateFor: nil, create: true)
            let data = try JSONSerialization.data(withJSONObject: json, options: [.prettyPrinted, .sortedKeys])
            try data.write(to: docs.appendingPathComponent(exportName), options: .atomic)
            print("[gm] layout saved \(active):\(summary) -> Documents/\(exportName)")
        } catch {
            print("[gm] layout export failed: \(error)")
        }
    }
}

/// Arranges the current preset's controls over the paused game: drag to move, pinch or the
/// slider to resize the selected one. The game draws the draft live underneath; Save keeps
/// it, Cancel restores the saved layout.
struct LayoutEditor: View {
    let model: GameModel
    let viewport: Viewport
    @State private var draft: ControlLayout
    @State private var selected: ControlKind?
    /// The dragging finger's offset from the control's center, so it doesn't jump.
    @State private var grab: CGSize?
    @State private var pinchStart: Float?

    private static let sizes: ClosedRange<Float> = 0.5...2
    private static let space = "layoutEditor"

    init(model: GameModel, viewport: Viewport) {
        self.model = model
        self.viewport = viewport
        _draft = State(initialValue: model.layouts[model.controlsKey]
            ?? defaultControlLayout(scheme: model.scheme, viewport: viewport))
    }

    var body: some View {
        ZStack {
            // Taps on empty space deselect; the pinch works anywhere.
            Color.black.opacity(0.3)
                .contentShape(Rectangle())
                .onTapGesture { selected = nil }
            ForEach(placeControls(scheme: model.scheme, layout: draft, viewport: viewport), id: \.kind) { control in
                handle(control)
            }
            toolbar
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .padding(.top, CGFloat(viewport.safeTop) + 8)
        }
        .ignoresSafeArea()
        // Full-screen like GameUIView, so this space is the game's view points.
        .coordinateSpace(.named(Self.space))
        .simultaneousGesture(
            MagnifyGesture()
                .onChanged { value in
                    guard let selected else { return }
                    let start = pinchStart ?? draft[keyPath: selected.path].size
                    pinchStart = start
                    setSize(start * Float(value.magnification), of: selected)
                }
                .onEnded { _ in pinchStart = nil }
        )
        .onChange(of: draft) { model.previewLayout(draft) }
    }

    private func handle(_ control: PlacedControl) -> some View {
        let isSelected = selected == control.kind
        let diameter = CGFloat(control.radius) * 2
        return Circle()
            .fill(control.kind.color.opacity(isSelected ? 0.35 : 0.18))
            .overlay(Circle().strokeBorder(isSelected ? .white : control.kind.color.opacity(0.8),
                                           lineWidth: isSelected ? 3 : 1.5))
            .overlay(
                Text(control.kind.label(model.scheme))
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(.white)
                    .fixedSize()
            )
            .frame(width: diameter, height: diameter)
            .contentShape(Circle())
            .gesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .named(Self.space))
                    .onChanged { drag in
                        selected = control.kind
                        let offset = grab ?? CGSize(width: CGFloat(control.x) - drag.startLocation.x,
                                                    height: CGFloat(control.y) - drag.startLocation.y)
                        grab = offset
                        move(control.kind, to: CGPoint(x: drag.location.x + offset.width,
                                                       y: drag.location.y + offset.height))
                    }
                    .onEnded { _ in grab = nil }
            )
            .position(x: CGFloat(control.x), y: CGFloat(control.y))
    }

    private var toolbar: some View {
        HStack(spacing: 14) {
            if let selected {
                Text(selected.label(model.scheme)).bold()
                Slider(value: Binding(
                    get: { draft[keyPath: selected.path].size },
                    set: { setSize($0, of: selected) }
                ), in: Self.sizes)
                .frame(width: 150)
                Text("\(Int((draft[keyPath: selected.path].size * 100).rounded()))%")
                    .font(.system(.body, design: .monospaced))
                    .frame(width: 52, alignment: .trailing)
            } else {
                Text("Drag to move · pinch to resize").foregroundStyle(.secondary)
            }
            Divider().frame(height: 22)
            Button("Reset") {
                draft = defaultControlLayout(scheme: model.scheme, viewport: viewport)
                selected = nil
            }
            Button("Cancel", role: .cancel) { model.finishLayoutEditing(saving: nil) }
            Button("Save") { model.finishLayoutEditing(saving: draft) }.bold()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.regularMaterial, in: Capsule())
    }

    /// Moves `kind`'s center to `point` (view points), kept inside the safe area.
    private func move(_ kind: ControlKind, to point: CGPoint) {
        let width = max(CGFloat(viewport.pointWidth - viewport.safeLeft - viewport.safeRight), 1)
        let height = max(CGFloat(viewport.pointHeight - viewport.safeTop - viewport.safeBottom), 1)
        let x = (point.x - CGFloat(viewport.safeLeft)) / width
        let y = (point.y - CGFloat(viewport.safeTop)) / height
        draft[keyPath: kind.path].x = Float(min(max(x, 0), 1))
        draft[keyPath: kind.path].y = Float(min(max(y, 0), 1))
    }

    private func setSize(_ size: Float, of kind: ControlKind) {
        draft[keyPath: kind.path].size = min(max(size, Self.sizes.lowerBound), Self.sizes.upperBound)
    }
}
