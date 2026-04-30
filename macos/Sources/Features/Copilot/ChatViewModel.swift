import Foundation
import SwiftUI
import GhosttyKit

struct ChatMessage: Identifiable {
    let id = UUID()
    let role: String
    var content: String
    var isStreaming: Bool = false
    var suggestedCommand: String?
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

        // Send a synthetic user message asking the LLM to comment on what it sees.
        // The sidecar will prepend the terminal context from context_buffer automatically.
        let autoMessages: [[String: Any]] = [
            ["role": "user", "content": "Based on my terminal output, is there an error or something I should fix? If yes, suggest the fix in one sentence with a command if applicable. If everything looks normal, respond with exactly: [no comment]"]
        ]

        bridge.send(
            method: "chat.send",
            params: [
                "provider": selectedProvider,
                "model": selectedModel,
                "messages": autoMessages,
                "max_tokens": 150,
                "system": "You are a developer assistant watching a terminal. ONLY speak when there is an actionable problem (error, failed command, misconfiguration). Suggest a FIX, not a description. If the terminal shows normal output (successful commands, prompts, navigation), respond with ONLY the text \"[no comment]\" and nothing else. Never narrate what the user is doing. Never describe environment variables unless they're causing an error. One sentence max.",
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

        default:
            break
        }
    }
}
