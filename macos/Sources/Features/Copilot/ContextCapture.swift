import Foundation
import GhosttyKit
import os

/// Captures terminal context and pushes it to the sidecar.
///
/// Uses a debounced timer to snapshot the visible terminal screen,
/// diff against the last known state, and send deltas via `context.push`.
///
/// Phase 2: grid-diff only. Phase 3+ will add OSC 133 hooks from Zig core.
final class ContextCapture {
    private let logger = Logger(subsystem: "com.fercervantes.decomptage", category: "ContextCapture")
    private let bridge: SidecarBridge

    private var timer: Timer?
    private var lastSnapshot: String = ""
    private var isEnabled: Bool = true
    private var isPaused: Bool = false  // pause during alt-screen

    /// Debounce interval in seconds
    private let debounceInterval: TimeInterval = 0.5

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

        bridge.send(
            method: "context.push",
            params: [
                "kind": "snapshot",
                "payload": ["text": delta],
            ]
        )
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
