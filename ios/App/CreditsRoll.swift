import SwiftUI

/// The credits, scrolling up once the derelict is cleared. Reads the bundled
/// `CREDITS.md`: `#` is the title, `##` a section, `-` a line; anything else is skipped.
/// A tap restarts at any time; once the roll ends it waits for one.
struct CreditsRoll: View {
    let restart: () -> Void
    @State private var offset: CGFloat = 0
    @State private var contentHeight: CGFloat = 0

    /// Scroll speed, points per second.
    private static let speed: CGFloat = 40

    var body: some View {
        GeometryReader { geo in
            VStack(spacing: 18) {
                Text("Derelict cleared").font(.largeTitle.bold())
                ForEach(Array(Self.lines.enumerated()), id: \.offset) { _, line in
                    switch line {
                    case .title(let text):
                        Text(text).font(.system(size: 44, weight: .heavy, design: .monospaced))
                            .padding(.top, 40)
                    case .section(let text):
                        Text(text.uppercased()).font(.caption.bold()).tracking(2).opacity(0.6)
                            .padding(.top, 24)
                    case .line(let text):
                        Text(text).font(.title3)
                    }
                }
                Text("Tap to start a new run").font(.title3).opacity(0.7).padding(.top, 60)
            }
            .multilineTextAlignment(.center)
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity)
            // Full height, taller than the screen, never squeezed to fit it.
            .fixedSize(horizontal: false, vertical: true)
            .background(GeometryReader { content in
                Color.clear.onAppear { contentHeight = content.size.height }
            })
            // Starts just below the screen and stops with the prompt centered.
            .offset(y: geo.size.height + offset)
            // Rolls once the content has been measured.
            .onChange(of: contentHeight) {
                let travel = geo.size.height / 2 + contentHeight
                withAnimation(.linear(duration: travel / Self.speed)) { offset = -travel }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(.black.opacity(0.85))
        .contentShape(Rectangle())
        .onTapGesture { restart() }
        .ignoresSafeArea()
    }

    private enum Line {
        case title(String), section(String), line(String)
    }

    private static let lines: [Line] = {
        guard let url = Bundle.main.url(forResource: "CREDITS", withExtension: "md"),
              let text = try? String(contentsOf: url, encoding: .utf8) else { return [] }
        return text.split(separator: "\n").compactMap { raw in
            let line = raw.trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("## ") { return .section(String(line.dropFirst(3))) }
            if line.hasPrefix("# ") { return .title(String(line.dropFirst(2))) }
            if line.hasPrefix("- ") { return .line(String(line.dropFirst(2))) }
            return nil
        }
    }()
}
