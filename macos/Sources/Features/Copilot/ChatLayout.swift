import Foundation
import SwiftUI
import GhosttyKit

/// The layout state for the per-tab copilot pane. This is owned by the
/// terminal view model (`BaseTerminalController`) and observed by
/// `TerminalView` so the split ratio and collapse state can drive the
/// SwiftUI layout without round-tripping through the Zig core.
///
/// Persistence: the initial values come from the Ghostty config
/// (`copilot-*` keys), and the user-facing ratio is persisted across
/// app restarts via `AppStorage` on the top-level window. Phase 1 will
/// replace `AppStorage` with proper per-tab session-restore state.
final class ChatLayout: ObservableObject {
    /// Whether the chat pane is currently hidden.
    @Published var collapsed: Bool

    /// The fraction of the tab occupied by the chat pane when expanded.
    /// Valid range is `[0.15, 0.75]`; this mirrors the clamp in
    /// `Config.finalize()` in the Zig core.
    @Published var ratio: CGFloat {
        didSet { ratio = min(0.75, max(0.15, ratio)) }
    }

    /// Which side of the tab the chat pane is docked on.
    @Published var position: Ghostty.Config.CopilotPosition

    init(
        collapsed: Bool = false,
        ratio: CGFloat = 0.33,
        position: Ghostty.Config.CopilotPosition = .right
    ) {
        self.collapsed = collapsed
        self.ratio = min(0.75, max(0.15, ratio))
        self.position = position
    }

    /// Build a layout seeded from the Ghostty config.
    convenience init(config: Ghostty.Config) {
        self.init(
            collapsed: config.copilotCollapsed,
            ratio: config.copilotRatio,
            position: config.copilotPosition
        )
    }

    func toggle() {
        collapsed.toggle()
    }
}
