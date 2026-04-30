import SwiftUI

struct ToolApprovalCard: View {
    let toolCallId: String
    let command: String
    let explanation: String
    let dangerLevel: String
    let onApprove: () -> Void
    let onDeny: () -> Void

    private var isDangerous: Bool {
        dangerLevel == "destructive"
    }

    private var isCaution: Bool {
        dangerLevel == "caution"
    }

    private var borderColor: Color {
        if isDangerous { return .red }
        if isCaution { return .orange }
        return Color(nsColor: .separatorColor)
    }

    private var backgroundColor: Color {
        if isDangerous { return .red.opacity(0.05) }
        if isCaution { return .orange.opacity(0.05) }
        return Color(nsColor: .controlBackgroundColor)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 4) {
                if isDangerous {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .font(.system(size: 10))
                }
                Text("Copilot wants to run:")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(.secondary)
            }

            Text(command)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.primary)
                .lineLimit(5)
                .frame(maxWidth: .infinity, alignment: .leading)

            if !explanation.isEmpty {
                Text(explanation)
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 8) {
                Button(action: onApprove) {
                    HStack(spacing: 3) {
                        Image(systemName: "checkmark.circle")
                            .font(.system(size: 10))
                        Text("Allow")
                            .font(.system(size: 10, weight: .medium))
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(isDangerous ? Color.red.opacity(0.1) : Color.green.opacity(0.1))
                    .cornerRadius(4)
                }
                .buttonStyle(.plain)

                Button(action: onDeny) {
                    HStack(spacing: 3) {
                        Image(systemName: "xmark.circle")
                            .font(.system(size: 10))
                        Text("Deny")
                            .font(.system(size: 10, weight: .medium))
                    }
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(Color(nsColor: .controlBackgroundColor))
                    .cornerRadius(4)
                }
                .buttonStyle(.plain)

                Spacer()
            }
        }
        .padding(8)
        .background(backgroundColor)
        .cornerRadius(6)
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .stroke(borderColor, lineWidth: isDangerous ? 1.5 : 0.5)
        )
    }
}
