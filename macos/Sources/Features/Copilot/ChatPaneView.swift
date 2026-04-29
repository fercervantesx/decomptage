import SwiftUI

/// The SwiftUI view for the per-tab copilot pane.
///
/// Phase 0: renders a solid color, header, and placeholder text to prove
/// the tab-level split composition works end-to-end. Phase 1 replaces
/// the body with a streaming message list + input + attachments.
struct ChatPaneView: View {
    @ObservedObject var layout: ChatLayout

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            placeholder
        }
        .frame(
            minWidth: 180,
            maxWidth: .infinity,
            minHeight: 120,
            maxHeight: .infinity
        )
        .background(Color(nsColor: .windowBackgroundColor))
        .accessibilityLabel("Copilot chat pane")
    }

    private var header: some View {
        HStack(spacing: 6) {
            Image(systemName: "sparkle")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
            Text("Copilot")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.primary)
            Spacer()
            Button {
                layout.toggle()
            } label: {
                Image(systemName: "sidebar.right")
                    .font(.system(size: 11))
            }
            .buttonStyle(.plain)
            .help("Hide the copilot pane")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }

    private var placeholder: some View {
        VStack(spacing: 8) {
            Spacer()
            Image(systemName: "bubble.left.and.bubble.right")
                .font(.system(size: 28, weight: .light))
                .foregroundStyle(.tertiary)
            Text("Copilot is not wired up yet.")
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
            Text("Phase 0 scaffold")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
