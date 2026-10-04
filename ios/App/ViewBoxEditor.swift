import SwiftUI

/// Persists the view boxes (landscape, portrait) in UserDefaults, and exports them on save
/// to `Documents/view-box.json` for reading off the device:
/// `xcrun devicectl device copy from --device <id> --domain-type appDataContainer
/// --domain-identifier com.jeffersonbutler.gm --source Documents/view-box.json --destination .`
enum ViewBoxStore {
    private static let key = "camera.viewBox"
    private static let exportName = "view-box.json"
    private static let names = ["landscape", "portrait"]

    /// Saved as `[orientation: [left, top, right, bottom]]`; a missing one is the default.
    static func load() -> [ViewBox] {
        let saved = UserDefaults.standard.dictionary(forKey: key) as? [String: [Double]] ?? [:]
        return names.enumerated().map { i, name in
            guard let v = saved[name], v.count == 4 else { return defaultViewBox(portrait: i == 1) }
            return ViewBox(left: Float(v[0]), top: Float(v[1]), right: Float(v[2]), bottom: Float(v[3]))
        }
    }

    static func save(_ boxes: [ViewBox]) {
        UserDefaults.standard.set(encode(boxes), forKey: key)
    }

    static func export(_ boxes: [ViewBox], viewport: Viewport) {
        let json: [String: Any] = [
            "savedAt": ISO8601DateFormatter().string(from: Date()),
            "units": "left, top, right, bottom: fractions of the safe area (0 = left/top edge, 1 = right/bottom)",
            "screenPt": ["width": Double(viewport.pointWidth), "height": Double(viewport.pointHeight)],
            "boxes": encode(boxes),
        ]
        do {
            let docs = try FileManager.default.url(for: .documentDirectory, in: .userDomainMask,
                                                   appropriateFor: nil, create: true)
            let data = try JSONSerialization.data(withJSONObject: json, options: [.prettyPrinted, .sortedKeys])
            try data.write(to: docs.appendingPathComponent(exportName), options: .atomic)
            print("[gm] view boxes saved \(encode(boxes)) -> Documents/\(exportName)")
        } catch {
            print("[gm] view box export failed: \(error)")
        }
    }

    private static func encode(_ boxes: [ViewBox]) -> [String: [Double]] {
        Dictionary(uniqueKeysWithValues: zip(names, boxes).map { name, b in
            (name, [b.left, b.top, b.right, b.bottom].map { (Double($0) * 1000).rounded() / 1000 })
        })
    }
}

/// Sets the current orientation's view box over the paused game: drag inside it to move
/// it, drag a corner to resize it. The camera follows the draft live underneath; Save
/// keeps it, Cancel restores the saved one.
struct ViewBoxEditor: View {
    let model: GameModel
    let viewport: Viewport
    @State private var draft: ViewBox
    /// The box as the current drag began, so a drag moves it by the finger's travel.
    @State private var dragStart: ViewBox?

    private static let space = "viewBoxEditor"
    /// The smallest a box gets, as a fraction of the safe area.
    private static let minSpan: Float = 0.05

    private var portrait: Bool { viewport.pointHeight > viewport.pointWidth }

    init(model: GameModel, viewport: Viewport) {
        self.model = model
        self.viewport = viewport
        _draft = State(initialValue: model.viewBoxes[viewport.pointHeight > viewport.pointWidth ? 1 : 0])
    }

    var body: some View {
        let rect = self.rect(draft)
        ZStack {
            Color.black.opacity(0.3)
            Rectangle()
                .fill(.cyan.opacity(0.12))
                .overlay(Rectangle().strokeBorder(.cyan, lineWidth: 2))
                .frame(width: rect.width, height: rect.height)
                .position(x: rect.midX, y: rect.midY)
                .gesture(drag { start, dx, dy in
                    let w = start.right - start.left, h = start.bottom - start.top
                    let left = min(max(start.left + dx, 0), 1 - w)
                    let top = min(max(start.top + dy, 0), 1 - h)
                    return ViewBox(left: left, top: top, right: left + w, bottom: top + h)
                })
            ForEach(0..<4, id: \.self) { corner in
                handle(corner, of: rect)
            }
            toolbar
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                .padding(.top, CGFloat(viewport.safeTop) + 8)
        }
        .ignoresSafeArea()
        .coordinateSpace(.named(Self.space))
        .onChange(of: draft) { model.applyViewBoxes(preview: (portrait, draft)) }
    }

    /// Corner 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
    private func handle(_ corner: Int, of rect: CGRect) -> some View {
        let right = corner % 2 == 1, bottom = corner >= 2
        return Circle()
            .fill(.white)
            .frame(width: 26, height: 26)
            .frame(width: 56, height: 56)
            .contentShape(Rectangle())
            .position(x: right ? rect.maxX : rect.minX, y: bottom ? rect.maxY : rect.minY)
            .gesture(drag { start, dx, dy in
                var b = start
                if right {
                    b.right = min(max(start.right + dx, start.left + Self.minSpan), 1)
                } else {
                    b.left = max(min(start.left + dx, start.right - Self.minSpan), 0)
                }
                if bottom {
                    b.bottom = min(max(start.bottom + dy, start.top + Self.minSpan), 1)
                } else {
                    b.top = max(min(start.top + dy, start.bottom - Self.minSpan), 0)
                }
                return b
            })
    }

    /// A drag that rebuilds the box from where it started and the finger's travel, in
    /// fractions of the safe area.
    private func drag(_ update: @escaping (ViewBox, Float, Float) -> ViewBox) -> some Gesture {
        DragGesture(minimumDistance: 0, coordinateSpace: .named(Self.space))
            .onChanged { value in
                let start = dragStart ?? draft
                dragStart = start
                let (w, h) = safeSize
                draft = update(start, Float(value.translation.width) / w, Float(value.translation.height) / h)
            }
            .onEnded { _ in dragStart = nil }
    }

    private var toolbar: some View {
        HStack(spacing: 14) {
            Text(portrait ? "Portrait view box" : "Landscape view box").bold()
            Text("Drag to move · corners to resize").foregroundStyle(.secondary)
            Divider().frame(height: 22)
            Button("Reset") { draft = defaultViewBox(portrait: portrait) }
            Button("Cancel", role: .cancel) { finish(saving: false) }
            Button("Save") { finish(saving: true) }.bold()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.regularMaterial, in: Capsule())
    }

    private func finish(saving: Bool) {
        if saving {
            model.viewBoxes[portrait ? 1 : 0] = draft
            ViewBoxStore.export(model.viewBoxes, viewport: viewport)
        }
        model.applyViewBoxes()
        model.editingViewBox = false
    }

    private var safeSize: (Float, Float) {
        (max(viewport.pointWidth - viewport.safeLeft - viewport.safeRight, 1),
         max(viewport.pointHeight - viewport.safeTop - viewport.safeBottom, 1))
    }

    /// `b` in view points.
    private func rect(_ b: ViewBox) -> CGRect {
        let (w, h) = safeSize
        return CGRect(x: CGFloat(viewport.safeLeft + b.left * w), y: CGFloat(viewport.safeTop + b.top * h),
                      width: CGFloat((b.right - b.left) * w), height: CGFloat((b.bottom - b.top) * h))
    }
}
