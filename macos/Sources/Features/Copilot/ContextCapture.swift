import Foundation
import GhosttyKit
import os

/// Captures terminal context and pushes it to the sidecar.
///
/// Uses a debounced timer to snapshot the visible terminal screen,
/// diff against the last known state, and send deltas via `context.push`.
///
/// Phase 2: grid-diff only. Phase 3+ will add OSC 133 hooks from Zig core.
final class ContextCapture: ObservableObject {
    private let logger = Logger(subsystem: "com.fercervantes.decomptage", category: "ContextCapture")
    private let bridge: SidecarBridge

    private var timer: Timer?
    private var lastSnapshot: String = ""
    private var isEnabled: Bool = true
    private var isPaused: Bool = false  // pause during alt-screen

    /// Published so the chat pane can show a warning banner
    @Published var secretsDetected: Bool = false

    /// Debounce interval in seconds
    private let debounceInterval: TimeInterval = 0.5

    /// Patterns that indicate secrets in terminal output
    private static let secretPatterns: [String] = [
        "password", "passwd", "token", "api_key", "api-key", "apikey",
        "secret", "credential", "private_key", "private-key",
        "AWS_SECRET", "ANTHROPIC_API_KEY", "OPENAI_API_KEY",
    ]

    /// Called after context is successfully pushed to the sidecar.
    /// Used by ChatViewModel to trigger auto-comments.
    var onContextPushed: (() -> Void)?

    /// Surface provider — returns the currently focused surface for reading
    var surfaceProvider: (() -> ghostty_surface_t?)?

    init(bridge: SidecarBridge = .shared) {
        self.bridge = bridge
    }

    func start() {
        guard timer == nil else { return }
        timer = Timer.scheduledTimer(withTimeInterval: debounceInterval, repeats: true) { [weak self] _ in
            self?.tick()
        }
        logger.info("context capture started (interval=\(self.debounceInterval)s)")
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    func setEnabled(_ enabled: Bool) {
        isEnabled = enabled
        if !enabled {
            lastSnapshot = ""
        }
    }

    func pause() {
        isPaused = true
    }

    func resume() {
        isPaused = false
    }

    /// Manually share the current screen content (user-triggered via keybind)
    func manualShare() {
        guard let surface = surfaceProvider?() else { return }
        let text = readVisibleScreen(surface: surface)
        guard !text.isEmpty else { return }

        bridge.send(
            method: "context.push",
            params: [
                "kind": "manual",
                "payload": ["text": text],
            ]
        )
        logger.info("manual context shared (\(text.count) chars)")
    }

    // MARK: - Private

    private func tick() {
        guard isEnabled, !isPaused, bridge.isConnected else { return }
        guard let surface = surfaceProvider?() else { return }

        let current = readVisibleScreen(surface: surface)
        guard !current.isEmpty, current != lastSnapshot else { return }

        // Alt-screen heuristic: if >80% of lines changed at once, it's likely
        // a TUI (vim, less, top). Suppress to avoid flooding the LLM.
        if !lastSnapshot.isEmpty {
            let oldLines = lastSnapshot.components(separatedBy: "\n")
            let newLines = current.components(separatedBy: "\n")
            let totalLines = max(oldLines.count, newLines.count)
            if totalLines > 5 {
                let oldSet = Set(oldLines)
                let changedCount = newLines.filter { !oldSet.contains($0) }.count
                let changeRatio = Double(changedCount) / Double(totalLines)
                if changeRatio > 0.8 {
                    // Likely alt-screen entered or exited — skip this tick
                    lastSnapshot = current
                    return
                }
            }
        }

        // Compute a simple diff: new lines added since last snapshot
        let delta: String
        if lastSnapshot.isEmpty {
            delta = current
        } else {
            let oldLines = Set(lastSnapshot.components(separatedBy: "\n"))
            let newLines = current.components(separatedBy: "\n")
            let added = newLines.filter { !oldLines.contains($0) && !$0.isEmpty }
            delta = added.joined(separator: "\n")
        }

        lastSnapshot = current

        guard !delta.isEmpty else { return }

        // Check for secrets before sending
        if containsSecrets(delta) {
            DispatchQueue.main.async {
                self.secretsDetected = true
            }
            logger.warning("secrets detected in terminal output — auto-context paused")
            return
        }

        DispatchQueue.main.async {
            self.secretsDetected = false
        }

        bridge.send(
            method: "context.push",
            params: [
                "kind": "snapshot",
                "payload": ["text": delta],
            ]
        )

        DispatchQueue.main.async {
            self.onContextPushed?()
        }
    }

    private func containsSecrets(_ text: String) -> Bool {
        let lower = text.lowercased()
        for pattern in Self.secretPatterns {
            let patLower = pattern.lowercased()
            if lower.contains(patLower) {
                // Only flag if it looks like an assignment (key=value or key: value or export KEY)
                for line in text.components(separatedBy: "\n") {
                    let lineLower = line.lowercased()
                    if lineLower.contains(patLower) &&
                       (line.contains("=") || line.contains("export ") || line.contains(": ")) {
                        return true
                    }
                }
            }
        }
        return false
    }

    private func readVisibleScreen(surface: ghostty_surface_t) -> String {
        // Read all visible text using a viewport-spanning selection
        var text = ghostty_text_s()
        let topLeft = ghostty_point_s(
            tag: GHOSTTY_POINT_VIEWPORT,
            coord: GHOSTTY_POINT_COORD_TOP_LEFT,
            x: 0,
            y: 0
        )
        let bottomRight = ghostty_point_s(
            tag: GHOSTTY_POINT_VIEWPORT,
            coord: GHOSTTY_POINT_COORD_BOTTOM_RIGHT,
            x: UInt32.max,
            y: UInt32.max
        )
        let sel = ghostty_selection_s(
            top_left: topLeft,
            bottom_right: bottomRight,
            rectangle: false
        )
        guard ghostty_surface_read_text(surface, sel, &text) else {
            return ""
        }
        defer { ghostty_surface_free_text(surface, &text) }
        guard let ptr = text.text else { return "" }
        return String(cString: ptr)
    }
}
