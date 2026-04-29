import Foundation
import AppKit
import UniformTypeIdentifiers

struct PendingAttachment: Identifiable {
    let id = UUID()
    let name: String
    let mimeType: String
    let data: Data
    let thumbnail: NSImage?

    var base64: String {
        data.base64EncodedString()
    }

    var isImage: Bool {
        mimeType.hasPrefix("image/")
    }
}

final class AttachmentHandler: ObservableObject {
    @Published var pending: [PendingAttachment] = []

    static let maxSize = 20 * 1024 * 1024 // 20MB

    func addFromURLs(_ urls: [URL]) {
        for url in urls {
            guard let data = try? Data(contentsOf: url) else { continue }
            guard data.count <= Self.maxSize else { continue }

            let name = url.lastPathComponent
            let mimeType = mimeTypeForURL(url)
            let thumbnail: NSImage? = mimeType.hasPrefix("image/")
                ? NSImage(data: data)
                : nil

            let attachment = PendingAttachment(
                name: name,
                mimeType: mimeType,
                data: data,
                thumbnail: thumbnail
            )
            pending.append(attachment)
        }
    }

    func addFromPasteboard(_ pasteboard: NSPasteboard) {
        if let items = pasteboard.readObjects(forClasses: [NSURL.self, NSImage.self]) {
            for item in items {
                if let url = item as? NSURL, let fileURL = url as URL? {
                    addFromURLs([fileURL])
                } else if let image = item as? NSImage,
                          let tiffData = image.tiffRepresentation,
                          let bitmap = NSBitmapImageRep(data: tiffData),
                          let pngData = bitmap.representation(using: .png, properties: [:]) {
                    let attachment = PendingAttachment(
                        name: "pasted-image.png",
                        mimeType: "image/png",
                        data: pngData,
                        thumbnail: image
                    )
                    pending.append(attachment)
                }
            }
        }
    }

    func remove(id: UUID) {
        pending.removeAll { $0.id == id }
    }

    func clear() {
        pending.removeAll()
    }

    /// Convert pending attachments to API-ready content parts
    func toContentParts() -> [[String: Any]] {
        pending.map { attachment in
            if attachment.isImage {
                return [
                    "type": "image",
                    "media_type": attachment.mimeType,
                    "data": attachment.base64,
                ]
            } else {
                // For non-image files, send as text with the content
                let text = String(data: attachment.data, encoding: .utf8)
                    ?? "[Binary file: \(attachment.name), \(attachment.data.count) bytes]"
                return [
                    "type": "text",
                    "text": "File: \(attachment.name)\n\(text)",
                ]
            }
        }
    }

    private func mimeTypeForURL(_ url: URL) -> String {
        if let type = UTType(filenameExtension: url.pathExtension) {
            return type.preferredMIMEType ?? "application/octet-stream"
        }
        return "application/octet-stream"
    }
}
