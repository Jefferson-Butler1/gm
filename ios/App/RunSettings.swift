import SwiftUI

/// Persists the run settings (difficulty + tuning, issue #15) in UserDefaults. Rust owns the
/// defaults (`defaultRunSettings()`); only values that were saved override them.
enum RunSettingsStore {
    private static let key = "run.settings"

    static let difficulties: [(value: Difficulty, key: String, label: String)] = [
        (.easy, "easy", "Easy"),
        (.normal, "normal", "Normal"),
        (.hard, "hard", "Hard"),
    ]
    private static let floats: [(key: String, path: WritableKeyPath<RunSettings, Float>)] = [
        ("moveSpeed", \.moveSpeed),
        ("rollDistance", \.rollDistance),
        ("rollSecs", \.rollSecs),
        ("rollIframeFraction", \.rollIframeFraction),
        ("fireRate", \.fireRate),
        ("ventSecs", \.ventSecs),
    ]
    /// Absent = the difficulty's value.
    private static let overrides: [(key: String, path: WritableKeyPath<RunSettings, Float?>)] = [
        ("enemyBulletSpeed", \.enemyBulletSpeed),
        ("shooterIntervalSecs", \.shooterIntervalSecs),
    ]

    static func load() -> RunSettings {
        var settings = defaultRunSettings()
        guard let saved = UserDefaults.standard.dictionary(forKey: key) else { return settings }
        if let value = saved["difficulty"] as? String,
           let found = difficulties.first(where: { $0.key == value }) {
            settings.difficulty = found.value
        }
        for field in floats {
            if let value = saved[field.key] as? Double { settings[keyPath: field.path] = Float(value) }
        }
        for field in overrides {
            if let value = saved[field.key] as? Double { settings[keyPath: field.path] = Float(value) }
        }
        if let value = saved["charges"] as? Int { settings.charges = UInt8(clamping: value) }
        if let value = saved["ventStyle"] as? String { settings.ventStyle = value == "regen" ? .regen : .clip }
        if let value = saved["spreadShooter"] as? Bool { settings.spreadShooter = value }
        return settings
    }

    static func save(_ settings: RunSettings) {
        var saved: [String: Any] = [:]
        saved["difficulty"] = difficulties.first { $0.value == settings.difficulty }?.key
        for field in floats { saved[field.key] = Double(settings[keyPath: field.path]) }
        for field in overrides {
            if let value = settings[keyPath: field.path] { saved[field.key] = Double(value) }
        }
        saved["charges"] = Int(settings.charges)
        saved["ventStyle"] = settings.ventStyle == .regen ? "regen" : "clip"
        saved["spreadShooter"] = settings.spreadShooter
        UserDefaults.standard.set(saved, forKey: key)
    }
}

/// Difficulty picker and the debug Tuning sliders. Everything applies when the next run
/// starts (death, win, or Restart run).
struct RunSettingsSections: View {
    @Bindable var model: GameModel

    var body: some View {
        Section {
            Picker("Difficulty", selection: $model.runSettings.difficulty) {
                ForEach(RunSettingsStore.difficulties, id: \.key) { option in
                    Text(option.label).tag(option.value)
                }
            }
            .pickerStyle(.segmented)
        } header: {
            Text("Difficulty")
        } footer: {
            Text("Enemy bullets and fire rate. Applies to the next run.")
        }

        let enemy = enemyDefaults(difficulty: model.runSettings.difficulty)
        Section {
            SliderRow(label: "Move speed", value: $model.runSettings.moveSpeed, range: 120...400, step: 5) {
                "\(Int($0)) pt/s"
            }
            SliderRow(label: "Roll distance", value: $model.runSettings.rollDistance, range: 60...240, step: 5) {
                "\(Int($0)) pt"
            }
            SliderRow(label: "Roll duration", value: $model.runSettings.rollSecs, range: 0.2...1.2, step: 0.05) {
                String(format: "%.2f s", $0)
            }
            SliderRow(label: "Roll i-frames", value: $model.runSettings.rollIframeFraction, range: 0...1, step: 0.05) {
                "first \(Int(($0 * 100).rounded()))%"
            }
            SliderRow(label: "Charges", value: charges, range: 1...20, step: 1) { "\(Int($0))" }
            SliderRow(label: "Fire cap", value: $model.runSettings.fireRate, range: 1...10, step: 0.25) {
                String(format: "%.2f shots/s", $0)
            }
            SliderRow(label: "Vent time", value: $model.runSettings.ventSecs, range: 0.2...3, step: 0.05) {
                String(format: "%.2f s", $0)
            }
            SliderRow(label: "Enemy bullet speed",
                      value: override(\.enemyBulletSpeed, default: enemy.enemyBulletSpeed),
                      range: 100...500, step: 10) { "\(Int($0)) pt/s" }
            SliderRow(label: "Shooter interval",
                      value: override(\.shooterIntervalSecs, default: enemy.shooterIntervalSecs),
                      range: 0.5...4, step: 0.1) { String(format: "%.1f s", $0) }
            Toggle("Vent style B (regenerating charges)", isOn: Binding(
                get: { model.runSettings.ventStyle == .regen },
                set: { model.runSettings.ventStyle = $0 ? .regen : .clip }))
            Toggle("Spread shooters", isOn: $model.runSettings.spreadShooter)
            Button("Reset to defaults") { model.resetTuning() }
        } header: {
            Text("Tuning")
        } footer: {
            Text("Applies on restart. Enemy values follow the difficulty until moved.")
        }
    }

    private var charges: Binding<Float> {
        Binding(get: { Float(model.runSettings.charges) },
                set: { model.runSettings.charges = UInt8($0.rounded()) })
    }

    /// Shows the difficulty's value until the slider moves, which sets an override.
    private func override(_ path: WritableKeyPath<RunSettings, Float?>, default value: Float) -> Binding<Float> {
        Binding(get: { model.runSettings[keyPath: path] ?? value },
                set: { model.runSettings[keyPath: path] = $0 })
    }
}

struct SliderRow: View {
    let label: String
    @Binding var value: Float
    let range: ClosedRange<Float>
    let step: Float
    let format: (Float) -> String

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack {
                Text(label)
                Spacer()
                Text(format(value)).monospacedDigit().foregroundStyle(.secondary)
            }
            Slider(value: $value, in: range, step: step)
        }
    }
}
