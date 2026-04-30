import Foundation
import SwiftUI
import GhosttyKit

struct ChatMessage: Identifiable {
    let id = UUID()
    let role: String
    var content: String
    var isStreaming: Bool = false
    var suggestedCommand: String?
    var toolApproval: ToolApprovalInfo?
}

struct ToolApprovalInfo {
    let toolCallId: String
    let command: String
    let explanation: String
    let dangerLevel: String
    var resolved: Bool = false
}

struct ProviderInfo: Identifiable, Hashable {
    let id: String
    let models: [String]
    let streaming: Bool
    let vision: Bool
    let configured: Bool
}

@MainActor
final class ChatViewModel: ObservableObject {
    @Published var messages: [ChatMessage] = []
    @Published var inputText: String = ""
    @Published var isStreaming: Bool = false
    @Published var error: String?

    @Published var providers: [ProviderInfo] = []
    @Published var selectedProvider: String {
        didSet { UserDefaults.standard.set(selectedProvider, forKey: "decomptage.selectedProvider") }
    }
    @Published var selectedModel: String {
        didSet { UserDefaults.standard.set(selectedModel, forKey: "decomptage.selectedModel") }
    }

    private let bridge = SidecarBridge.shared
    let contextCapture = ContextCapture()

    /// Set this to the focused terminal surface so context capture can read it.
    /// Typically wired from the TerminalView's @FocusedValue.
    var focusedSurface: ghostty_surface_t? {
        didSet {
            contextCapture.surfaceProvider = { [weak self] in
                self?.focusedSurface
            }
        }
    }

    init() {
        self.selectedProvider = UserDefaults.standard.string(forKey: "decomptage.selectedProvider") ?? "anthropic"
        self.selectedModel = UserDefaults.standard.string(forKey: "decomptage.selectedModel") ?? "claude-sonnet-4-6"

        bridge.onNotification = { [weak self] method, params in
            Task { @MainActor in
                self?.handleNotification(method: method, params: params)
            }
        }
    }

    /// When true, the copilot proactively comments on terminal activity
    @Published var autoComment: Bool = true

    /// Debounce timer for auto-comments after context arrives
    private var autoCommentTimer: Timer?
    private let autoCommentDelay: TimeInterval = 5.0

    /// Minimum time between auto-comments (prevents flooding)
    private var lastAutoCommentTime: Date = .distantPast
    private let autoCommentCooldown: TimeInterval = 15.0

    /// Auto-commenting only activates after the first terminal context push
    /// (i.e., the user typed something and the screen changed). Silent during
    /// shell startup / init noise.
    private var terminalHasActivity: Bool = false

    func connect() {
        bridge.start()
        fetchProviders()
        contextCapture.start()

        // Listen for context pushes and trigger auto-comments
        contextCapture.onContextPushed = { [weak self] in
            self?.scheduleAutoComment()
        }
    }

    func clearConversation() {
        messages.removeAll()
        error = nil
    }

    func send() {
        let text = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }

        inputText = ""
        error = nil

        messages.append(ChatMessage(role: "user", content: text))
        messages.append(ChatMessage(role: "assistant", content: "", isStreaming: true))
        isStreaming = true

        let apiMessages = messages.dropLast().map { msg -> [String: Any] in
            ["role": msg.role, "content": msg.content]
        }

        bridge.send(
            method: "chat.send",
            params: [
                "provider": selectedProvider,
                "model": selectedModel,
                "messages": Array(apiMessages),
                "max_tokens": 4096,
            ]
        ) { [weak self] result in
            if case .failure(let err) = result {
                Task { @MainActor in
                    self?.error = err.localizedDescription
                    self?.isStreaming = false
                }
            }
        }
    }

    var availableModels: [String] {
        providers.first(where: { $0.id == selectedProvider })?.models ?? []
    }

    // MARK: - Tool Actions

    func approveToolCall(id: String) {
        bridge.send(
            method: "chat.tool_approval_response",
            params: ["tool_call_id": id, "approved": true]
        )
        if let idx = messages.lastIndex(where: { $0.toolApproval?.toolCallId == id }) {
            messages[idx].toolApproval?.resolved = true

            // Execute the command in the terminal
            let command = messages[idx].toolApproval!.command
            executeCommandInTerminal(command: command, toolCallId: id)
        }
    }

    private func executeCommandInTerminal(command: String, toolCallId: String) {
        guard let surface = focusedSurface else {
            bridge.send(
                method: "chat.run_command_result",
                params: ["tool_call_id": toolCallId, "output": "No terminal surface available", "exit_code": -1]
            )
            return
        }

        // Write command + newline to PTY (executes it)
        let commandWithNewline = command + "\n"
        ghostty_surface_text(surface, commandWithNewline, UInt(commandWithNewline.utf8.count))

        // Wait for output to settle, then read the screen and send it back
        DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) { [weak self] in
            guard let self = self else { return }
            let output = self.readTerminalForTool()
            self.bridge.send(
                method: "chat.run_command_result",
                params: [
                    "tool_call_id": toolCallId,
                    "output": output,
                    "exit_code": 0,  // We can't easily get exit code without shell integration
                ]
            )
        }
    }

    func denyToolCall(id: String) {
        bridge.send(
            method: "chat.tool_approval_response",
            params: ["tool_call_id": id, "approved": false]
        )
        if let idx = messages.lastIndex(where: { $0.toolApproval?.toolCallId == id }) {
            messages[idx].toolApproval?.resolved = true
            messages[idx].content = "(Command denied by user)"
        }
    }

    private func readTerminalForTool() -> String {
        guard let surface = focusedSurface else { return "" }
        // Reuse the same read logic as ContextCapture
        var text = ghostty_text_s()
        let topLeft = ghostty_point_s(
            tag: GHOSTTY_POINT_VIEWPORT,
            coord: GHOSTTY_POINT_COORD_TOP_LEFT,
            x: 0, y: 0
        )
        let bottomRight = ghostty_point_s(
            tag: GHOSTTY_POINT_VIEWPORT,
            coord: GHOSTTY_POINT_COORD_BOTTOM_RIGHT,
            x: UInt32.max, y: UInt32.max
        )
        let sel = ghostty_selection_s(
            top_left: topLeft,
            bottom_right: bottomRight,
            rectangle: false
        )
        guard ghostty_surface_read_text(surface, sel, &text) else { return "" }
        defer { ghostty_surface_free_text(surface, &text) }
        guard let ptr = text.text else { return "" }
        return String(cString: ptr)
    }

    // MARK: - Auto-comment

    private func scheduleAutoComment() {
        autoCommentTimer?.invalidate()

        // Skip the very first context push (shell init noise).
        // Activate on the second push onward — that means the user has typed.
        if !terminalHasActivity {
            terminalHasActivity = true
            return
        }

        guard autoComment else { return }

        // Enforce cooldown — don't auto-comment more than once per 30s
        let timeSinceLastComment = Date().timeIntervalSince(lastAutoCommentTime)
        if timeSinceLastComment < autoCommentCooldown {
            return
        }

        autoCommentTimer = Timer.scheduledTimer(withTimeInterval: autoCommentDelay, repeats: false) { [weak self] _ in
            Task { @MainActor in
                self?.sendAutoComment()
            }
        }
    }

    private func sendAutoComment() {
        guard autoComment, !isStreaming, bridge.isConnected else { return }

        // Enforce cooldown again (timer may have been scheduled before cooldown was checked)
        let timeSinceLastComment = Date().timeIntervalSince(lastAutoCommentTime)
        if timeSinceLastComment < autoCommentCooldown {
            return
        }

        // Don't auto-comment if currently streaming or user just typed
        if let last = messages.last, last.role == "user" || last.isStreaming {
            return
        }

        lastAutoCommentTime = Date()

        messages.append(ChatMessage(role: "assistant", content: "", isStreaming: true))
        isStreaming = true

        // Build the auto-comment request based on conversation state.
        // If there's an active conversation, continue it naturally.
        // If no conversation, only comment on errors.
        let hasConversation = !messages.isEmpty
        let systemPrompt: String
        let userPrompt: String

        if hasConversation {
            // Active conversation — continue guiding based on what happened
            systemPrompt = "You are a senior developer assistant embedded in a terminal. You're helping the user with a task. You just received new terminal output showing what happened after the last interaction. Continue the conversation naturally: acknowledge what happened, point out issues, or suggest the next step. Be concise (2-3 sentences). Use ```sh code blocks for commands."
            userPrompt = "Here's what just happened in my terminal. What should I do next? If nothing interesting happened, say [no comment]."
        } else {
            // No conversation — strict error-only mode
            systemPrompt = "You are a developer assistant watching a terminal. Only respond if there's a clear error, failed command, or problem that needs fixing. Explain the issue and suggest a fix. If everything looks normal, respond with exactly: [no comment]"
            userPrompt = "Is there anything wrong in my terminal that I should fix? If not, say [no comment]."
        }

        // Include conversation history so the LLM has context of what it previously suggested
        var autoMessages: [[String: Any]] = messages.map { msg in
            ["role": msg.role, "content": msg.content] as [String: Any]
        }
        autoMessages.append(["role": "user", "content": userPrompt])

        bridge.send(
            method: "chat.send",
            params: [
                "provider": selectedProvider,
                "model": selectedModel,
                "messages": autoMessages,
                "max_tokens": 512,
                "system": systemPrompt,
            ]
        ) { [weak self] result in
            if case .failure(let err) = result {
                Task { @MainActor in
                    self?.isStreaming = false
                    // Silently remove the empty assistant message on failure
                    if self?.messages.last?.content.isEmpty == true {
                        self?.messages.removeLast()
                    }
                }
            }
        }
    }

    // MARK: - Private

    private func fetchProviders() {
        bridge.send(method: "session.hello") { [weak self] result in
            guard let self = self else { return }
            Task { @MainActor in
                if case .success(let value) = result,
                   let dict = value as? [String: Any],
                   let providersArray = dict["providers"] as? [[String: Any]] {
                    self.providers = providersArray.compactMap { p in
                        guard let id = p["id"] as? String,
                              let models = p["models"] as? [String] else { return nil }
                        return ProviderInfo(
                            id: id,
                            models: models,
                            streaming: p["streaming"] as? Bool ?? false,
                            vision: p["vision"] as? Bool ?? false,
                            configured: p["configured"] as? Bool ?? false
                        )
                    }

                    // Validate current selection
                    if !self.providers.contains(where: { $0.id == self.selectedProvider }) {
                        if let first = self.providers.first(where: { $0.configured }) {
                            self.selectedProvider = first.id
                            self.selectedModel = first.models.first ?? ""
                        }
                    }
                }
            }
        }
    }

    private func handleNotification(method: String, params: [String: Any]) {
        switch method {
        case "chat.delta":
            if let text = params["text"] as? String,
               messages.last?.role == "assistant" {
                messages[messages.count - 1].content += text
            }

        case "chat.done":
            if messages.last?.role == "assistant" {
                messages[messages.count - 1].isStreaming = false
                // If the LLM said "[no comment]" or was empty, remove silently
                let content = messages[messages.count - 1].content
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                if content.isEmpty || content == "[no comment]" || content.lowercased().contains("[no comment]") {
                    messages.removeLast()
                }
            }
            isStreaming = false

        case "chat.error":
            let msg = params["message"] as? String ?? "Unknown error"
            error = msg
            isStreaming = false
            if messages.last?.isStreaming == true {
                messages.removeLast()
            }

        case "chat.suggested_command":
            if let command = params["command"] as? String {
                if messages.last?.role == "assistant" {
                    messages[messages.count - 1].suggestedCommand = command
                }
            }

        case "chat.tool_approval_request":
            if let toolCallId = params["tool_call_id"] as? String,
               let command = params["command"] as? String {
                let explanation = params["explanation"] as? String ?? ""
                let dangerLevel = params["danger_level"] as? String ?? "safe"
                let approval = ToolApprovalInfo(
                    toolCallId: toolCallId,
                    command: command,
                    explanation: explanation,
                    dangerLevel: dangerLevel
                )
                messages.append(ChatMessage(
                    role: "assistant",
                    content: "",
                    toolApproval: approval
                ))
            }

        case "chat.read_terminal_request":
            if let requestId = params["request_id"] as? String {
                let text = readTerminalForTool()
                bridge.send(
                    method: "chat.read_terminal_response",
                    params: ["request_id": requestId, "text": text]
                )
            }

        default:
            break
        }
    }
}
