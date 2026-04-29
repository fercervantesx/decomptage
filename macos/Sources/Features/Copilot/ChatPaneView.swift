import SwiftUI

struct ChatPaneView: View {
    @ObservedObject var layout: ChatLayout
    @StateObject private var viewModel = ChatViewModel()

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            messageList
            Divider()
            inputArea
        }
        .frame(
            minWidth: 180,
            maxWidth: .infinity,
            minHeight: 120,
            maxHeight: .infinity
        )
        .background(Color(nsColor: .windowBackgroundColor))
        .accessibilityLabel("Copilot chat pane")
        .onAppear {
            viewModel.connect()
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack(spacing: 6) {
            Image(systemName: "sparkle")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
            Text("Copilot")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.primary)

            if viewModel.isStreaming {
                ProgressView()
                    .controlSize(.mini)
                    .padding(.leading, 2)
            }

            Spacer()

            Circle()
                .fill(SidecarBridge.shared.isConnected ? Color.green : Color.red)
                .frame(width: 6, height: 6)
                .help(SidecarBridge.shared.isConnected ? "Connected to sidecar" : "Disconnected")

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

    // MARK: - Messages

    private var messageList: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 8) {
                    ForEach(viewModel.messages) { message in
                        MessageBubble(message: message)
                            .id(message.id)
                    }

                    if let error = viewModel.error {
                        HStack(spacing: 4) {
                            Image(systemName: "exclamationmark.triangle.fill")
                                .foregroundStyle(.red)
                                .font(.system(size: 10))
                            Text(error)
                                .font(.system(size: 11))
                                .foregroundStyle(.red)
                        }
                        .padding(.horizontal, 10)
                    }
                }
                .padding(.vertical, 8)
            }
            .onChange(of: viewModel.messages.count) { _ in
                if let last = viewModel.messages.last {
                    withAnimation(.easeOut(duration: 0.15)) {
                        proxy.scrollTo(last.id, anchor: .bottom)
                    }
                }
            }
        }
    }

    // MARK: - Input

    private var inputArea: some View {
        HStack(spacing: 6) {
            TextField("Ask the copilot...", text: $viewModel.inputText)
                .textFieldStyle(.plain)
                .font(.system(size: 12))
                .onSubmit {
                    viewModel.send()
                }
                .disabled(viewModel.isStreaming)

            Button {
                viewModel.send()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.system(size: 16))
                    .foregroundColor(
                        viewModel.inputText.trimmingCharacters(in: .whitespaces).isEmpty
                            ? Color.gray : Color.blue
                    )
            }
            .buttonStyle(.plain)
            .disabled(viewModel.inputText.trimmingCharacters(in: .whitespaces).isEmpty || viewModel.isStreaming)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
    }
}

// MARK: - Message Bubble

private struct MessageBubble: View {
    let message: ChatMessage

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 4) {
                Text(message.role == "user" ? "You" : "Copilot")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.secondary)

                if message.isStreaming {
                    Image(systemName: "circle.fill")
                        .font(.system(size: 4))
                        .foregroundStyle(.green)
                }
            }

            Text(message.content.isEmpty && message.isStreaming ? "..." : message.content)
                .font(.system(size: 12))
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)

            if let command = message.suggestedCommand {
                SuggestedCommandCard(command: command)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .background(
            message.role == "user"
                ? Color.accentColor.opacity(0.05)
                : Color.clear
        )
    }
}

// MARK: - Suggested Command Card

private struct SuggestedCommandCard: View {
    let command: String

    @FocusedValue(\.ghosttySurfaceView) private var focusedSurface

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "terminal")
                .font(.system(size: 10))
                .foregroundStyle(.secondary)

            Text(command)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.primary)
                .lineLimit(3)

            Spacer()

            Button {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(command, forType: .string)
            } label: {
                Image(systemName: "doc.on.doc")
                    .font(.system(size: 10))
            }
            .buttonStyle(.plain)
            .help("Copy command")

            Button {
                guard let surface = focusedSurface?.surface else { return }
                CommandInsertion.insert(command: command, into: surface)
            } label: {
                Image(systemName: "arrow.right.circle")
                    .font(.system(size: 10))
            }
            .buttonStyle(.plain)
            .help("Insert into terminal (→)")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .background(Color(nsColor: .controlBackgroundColor))
        .cornerRadius(6)
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .stroke(Color(nsColor: .separatorColor), lineWidth: 0.5)
        )
    }
}
