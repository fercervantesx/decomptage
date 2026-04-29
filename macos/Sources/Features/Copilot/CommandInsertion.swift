import Foundation
import GhosttyKit

/// Handles inserting a suggested command into the active terminal's PTY
/// without executing it (no trailing newline).
enum CommandInsertion {
    /// Insert the given command text into the provided surface's PTY input.
    /// The command is written WITHOUT a trailing newline, so the user sees
    /// it at their prompt and can review/edit before pressing Enter.
    static func insert(command: String, into surface: ghostty_surface_t) {
        ghostty_surface_text(surface, command, UInt(command.utf8.count))
    }
}
