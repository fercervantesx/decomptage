import SwiftUI
import GhosttyKit

/// Composes the terminal split tree with the copilot chat pane inside a
/// single tab. This is a tab-level split that lives ABOVE the terminal's
/// `SplitTree<Ghostty.SurfaceView>`, so the PTY split tree is unaffected.
///
/// When `ghostty.config.copilotEnabled == false` or the chat pane is
/// collapsed, the terminal content fills the whole tab.
///
/// The orientation (horizontal vs vertical split) and the side the chat
/// pane sits on are driven by `chatLayout.position`.
struct TerminalWithCopilotSplit<Terminal: View>: View {
    @ObservedObject var ghostty: Ghostty.App
    @ObservedObject var chatLayout: ChatLayout
    let terminal: () -> Terminal

    var body: some View {
        if !ghostty.config.copilotEnabled || chatLayout.collapsed {
            terminal()
        } else {
            splitView
        }
    }

    @ViewBuilder
    private var splitView: some View {
        if chatLayout.position.isVertical {
            HSplitView {
                if chatLayout.position.isTrailing {
                    terminal()
                    chatPane
                } else {
                    chatPane
                    terminal()
                }
            }
        } else {
            VSplitView {
                if chatLayout.position.isTrailing {
                    terminal()
                    chatPane
                } else {
                    chatPane
                    terminal()
                }
            }
        }
    }

    private var chatPane: some View {
        ChatPaneView(layout: chatLayout)
    }
}
