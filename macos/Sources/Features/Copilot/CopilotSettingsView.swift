import SwiftUI

struct CopilotSettingsView: View {
    @Environment(\.dismiss) private var dismiss

    @AppStorage("decomptage.selectedProvider") private var selectedProvider = "anthropic"
    @AppStorage("decomptage.selectedModel") private var selectedModel = "claude-sonnet-4-6"
    @AppStorage("decomptage.contextAutoCapture") private var contextAutoCapture = true
    @AppStorage("decomptage.contextDebounceMs") private var contextDebounceMs = 500
    @AppStorage("decomptage.pauseAltScreen") private var pauseAltScreen = true
    @AppStorage("decomptage.copilotPosition") private var copilotPosition = "right"
    @AppStorage("decomptage.copilotRatio") private var copilotRatio = 0.33

    // API keys stored in UserDefaults for now; keychain integration in Phase 7+
    @AppStorage("decomptage.anthropicApiKey") private var anthropicApiKey = ""
    @AppStorage("decomptage.openaiApiKey") private var openaiApiKey = ""
    @AppStorage("decomptage.geminiApiKey") private var geminiApiKey = ""
    @AppStorage("decomptage.ollamaUrl") private var ollamaUrl = "http://localhost:11434"

    // AWS Bedrock credentials
    @AppStorage("decomptage.awsAccessKeyId") private var awsAccessKeyId = ""
    @AppStorage("decomptage.awsSecretAccessKey") private var awsSecretAccessKey = ""
    @AppStorage("decomptage.awsRegion") private var awsRegion = "us-east-1"
    @AppStorage("decomptage.bedrockApiKey") private var bedrockApiKey = ""

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    providerSection
                    contextSection
                    layoutSection
                }
                .padding(16)
            }
        }
        .frame(width: 420, height: 520)
    }

    private var header: some View {
        HStack {
            Text("Copilot Settings")
                .font(.system(size: 13, weight: .semibold))
            Spacer()
            Button("Done") {
                pushCredentialsToSidecar()
                dismiss()
            }
            .keyboardShortcut(.defaultAction)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    private func pushCredentialsToSidecar() {
        var vars: [String: Any] = [:]
        if !anthropicApiKey.isEmpty { vars["ANTHROPIC_API_KEY"] = anthropicApiKey }
        if !openaiApiKey.isEmpty { vars["OPENAI_API_KEY"] = openaiApiKey }
        if !geminiApiKey.isEmpty { vars["GEMINI_API_KEY"] = geminiApiKey }
        if !awsAccessKeyId.isEmpty { vars["AWS_ACCESS_KEY_ID"] = awsAccessKeyId }
        if !awsSecretAccessKey.isEmpty { vars["AWS_SECRET_ACCESS_KEY"] = awsSecretAccessKey }
        if !awsRegion.isEmpty { vars["AWS_REGION"] = awsRegion }
        if !bedrockApiKey.isEmpty { vars["BEDROCK_API_KEY"] = bedrockApiKey }
        if !ollamaUrl.isEmpty { vars["OLLAMA_URL"] = ollamaUrl }

        guard !vars.isEmpty else { return }
        SidecarBridge.shared.send(method: "settings.update", params: vars)
    }

    // MARK: - Providers

    private var providerSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Providers", systemImage: "cloud")
                .font(.system(size: 12, weight: .semibold))

            GroupBox {
                VStack(alignment: .leading, spacing: 8) {
                    apiKeyField(label: "Anthropic API Key", binding: $anthropicApiKey, placeholder: "sk-ant-...")
                    apiKeyField(label: "OpenAI API Key", binding: $openaiApiKey, placeholder: "sk-...")
                    apiKeyField(label: "Gemini API Key", binding: $geminiApiKey, placeholder: "AI...")

                    HStack {
                        Text("Ollama URL")
                            .font(.system(size: 11))
                            .frame(width: 110, alignment: .leading)
                        TextField("http://localhost:11434", text: $ollamaUrl)
                            .textFieldStyle(.roundedBorder)
                            .font(.system(size: 11))
                    }

                    Divider().padding(.vertical, 4)

                    Text("AWS Bedrock")
                        .font(.system(size: 11, weight: .medium))
                    apiKeyField(label: "Bedrock API Key", binding: $bedrockApiKey, placeholder: "bedrock-api-key-...")
                    Text("— or use IAM credentials —")
                        .font(.system(size: 9))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .center)
                    apiKeyField(label: "Access Key ID", binding: $awsAccessKeyId, placeholder: "AKIA...")
                    apiKeyField(label: "Secret Access Key", binding: $awsSecretAccessKey, placeholder: "...")
                    HStack {
                        Text("Region")
                            .font(.system(size: 11))
                            .frame(width: 110, alignment: .leading)
                        TextField("us-east-1", text: $awsRegion)
                            .textFieldStyle(.roundedBorder)
                            .font(.system(size: 11))
                    }
                }
                .padding(4)
            }

            Text("API keys are stored locally. Ollama requires no key. Bedrock uses AWS credentials (Access Key ID + Secret Key, or `aws configure`).")
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
        }
    }

    private func apiKeyField(label: String, binding: Binding<String>, placeholder: String) -> some View {
        HStack {
            Text(label)
                .font(.system(size: 11))
                .frame(width: 110, alignment: .leading)
            SecureField(placeholder, text: binding)
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 11))
        }
    }

    // MARK: - Context Capture

    private var contextSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Context Capture", systemImage: "eye")
                .font(.system(size: 12, weight: .semibold))

            GroupBox {
                VStack(alignment: .leading, spacing: 8) {
                    Toggle("Auto-capture terminal output", isOn: $contextAutoCapture)
                        .font(.system(size: 11))

                    HStack {
                        Text("Debounce interval")
                            .font(.system(size: 11))
                        Slider(value: Binding(
                            get: { Double(contextDebounceMs) },
                            set: { contextDebounceMs = Int($0) }
                        ), in: 100...5000, step: 100)
                        Text("\(contextDebounceMs)ms")
                            .font(.system(size: 10, design: .monospaced))
                            .frame(width: 50)
                    }

                    Toggle("Pause during alternate screen (vim, less, etc.)", isOn: $pauseAltScreen)
                        .font(.system(size: 11))
                }
                .padding(4)
            }

            Text("When enabled, terminal output is automatically shared with the LLM so it can see what you're doing.")
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
        }
    }

    // MARK: - Layout

    private var layoutSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Layout", systemImage: "rectangle.split.2x1")
                .font(.system(size: 12, weight: .semibold))

            GroupBox {
                VStack(alignment: .leading, spacing: 8) {
                    Picker("Pane position", selection: $copilotPosition) {
                        Text("Right").tag("right")
                        Text("Left").tag("left")
                        Text("Bottom").tag("bottom")
                        Text("Top").tag("top")
                    }
                    .pickerStyle(.segmented)
                    .font(.system(size: 11))

                    HStack {
                        Text("Pane size")
                            .font(.system(size: 11))
                        Slider(value: $copilotRatio, in: 0.15...0.75, step: 0.05)
                        Text("\(Int(copilotRatio * 100))%")
                            .font(.system(size: 10, design: .monospaced))
                            .frame(width: 35)
                    }
                }
                .padding(4)
            }
        }
    }
}
