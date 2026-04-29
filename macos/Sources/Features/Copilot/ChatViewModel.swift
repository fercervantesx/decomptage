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
    private let contextCapture = ContextCapture()

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

    func connect() {
        bridge.start()
        fetchProviders()
        contextCapture.start()
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
