import Foundation
import os

/// Manages the lifecycle of the decomptage-sidecar process and provides
/// a JSON-RPC client over Unix-domain socket for communicating with it.
///
/// One bridge per app (singleton); multiple tabs share the same sidecar.
final class SidecarBridge: ObservableObject {
    static let shared = SidecarBridge()

    @Published private(set) var isConnected = false
    @Published private(set) var sessionId: String?

    private let logger = Logger(subsystem: "com.fercervantes.decomptage", category: "SidecarBridge")

    private var process: Process?
    private var inputStream: InputStream?
    private var outputStream: OutputStream?
    private var readBuffer = Data()

    private let socketPath: String = {
        let cacheDir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first!
        return cacheDir.appendingPathComponent("decomptage/sidecar.sock").path
    }()

    /// Callbacks keyed by JSON-RPC request id
    private var pendingRequests: [Int: (Result<Any, Error>) -> Void] = [:]
    private var nextRequestId = 1

    /// Stream notification handler — called on every server-pushed notification
    var onNotification: ((String, [String: Any]) -> Void)?

    private init() {}

    private var reconnectAttempt = 0
    private var reconnectTimer: Timer?
    private var shouldReconnect = true

    // MARK: - Lifecycle

    func start() {
        shouldReconnect = true
        ensureSidecarRunning()
        connectToSocket()
    }

    func stop() {
        shouldReconnect = false
        reconnectTimer?.invalidate()
        reconnectTimer = nil
        disconnect()
        terminateSidecar()
    }

    private func scheduleReconnect() {
        guard shouldReconnect else { return }
        reconnectAttempt += 1
        let delay = min(Double(reconnectAttempt) * 0.5, 5.0) // 0.5s, 1s, 1.5s, ... cap at 5s
        logger.info("scheduling reconnect in \(delay)s (attempt \(self.reconnectAttempt))")

        reconnectTimer?.invalidate()
        reconnectTimer = Timer.scheduledTimer(withTimeInterval: delay, repeats: false) { [weak self] _ in
            guard let self = self else { return }
            self.ensureSidecarRunning()
            self.connectToSocket()
        }
    }

    // MARK: - Sidecar Process

    private func ensureSidecarRunning() {
        // Check if already running via socket
        if FileManager.default.fileExists(atPath: socketPath) {
            // Try connecting — if it works, sidecar is alive
            return
        }

        let sidecarPath = findSidecarBinary()
        guard let path = sidecarPath else {
            logger.error("decomptage-sidecar binary not found")
            return
        }

        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: path)
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice

        do {
            try proc.run()
            self.process = proc
            logger.info("sidecar spawned pid=\(proc.processIdentifier)")
            // Give it a moment to bind the socket
            Thread.sleep(forTimeInterval: 0.3)
        } catch {
            logger.error("failed to spawn sidecar: \(error)")
        }
    }

    private func findSidecarBinary() -> String? {
        // 1. Inside app bundle
        if let bundlePath = Bundle.main.path(forAuxiliaryExecutable: "decomptage-sidecar") {
            return bundlePath
        }

        // 2. In the Cargo build output (dev mode)
        let devPath = URL(fileURLWithPath: #file)
            .deletingLastPathComponent() // Copilot/
            .deletingLastPathComponent() // Features/
            .deletingLastPathComponent() // Sources/
            .deletingLastPathComponent() // macos/
            .appendingPathComponent("sidecar/target/debug/decomptage-sidecar")
            .path
        if FileManager.default.isExecutableFile(atPath: devPath) {
            return devPath
        }

        // 3. In PATH
        let whichResult = shell("which decomptage-sidecar")
        if !whichResult.isEmpty {
            return whichResult.trimmingCharacters(in: .whitespacesAndNewlines)
        }

        return nil
    }

    private func terminateSidecar() {
        process?.terminate()
        process = nil
    }

    // MARK: - Socket Connection

    private func connectToSocket() {
        guard FileManager.default.fileExists(atPath: socketPath) else {
            logger.warning("socket not found at \(self.socketPath)")
            return
        }

        let addr = sockaddr_un.make(path: socketPath)
        let fd = socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else {
            logger.error("socket() failed")
            return
        }

        var addrCopy = addr
        let connectResult = withUnsafePointer(to: &addrCopy) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                connect(fd, sockPtr, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }

        guard connectResult == 0 else {
            logger.error("connect() failed: \(errno)")
            Darwin.close(fd)
            return
        }

        // Wrap in streams
        var readStream: Unmanaged<CFReadStream>?
        var writeStream: Unmanaged<CFWriteStream>?
        CFStreamCreatePairWithSocket(nil, fd, &readStream, &writeStream)

        guard let input = readStream?.takeRetainedValue() as InputStream?,
              let output = writeStream?.takeRetainedValue() as OutputStream? else {
            logger.error("failed to create streams from socket")
            Darwin.close(fd)
            return
        }

        // Don't close fd when streams close — we manage it
        input.setProperty(kCFBooleanTrue, forKey: Stream.PropertyKey(rawValue: kCFStreamPropertyShouldCloseNativeSocket as String))
        output.setProperty(kCFBooleanTrue, forKey: Stream.PropertyKey(rawValue: kCFStreamPropertyShouldCloseNativeSocket as String))

        self.inputStream = input
        self.outputStream = output

        input.open()
        output.open()

        DispatchQueue.main.async { self.isConnected = true }
        reconnectAttempt = 0
        logger.info("connected to sidecar")

        // Start reading in background
        startReading()

        // Handshake
        sendHello()
    }

    private func disconnect() {
        inputStream?.close()
        outputStream?.close()
        inputStream = nil
        outputStream = nil
        DispatchQueue.main.async {
            self.isConnected = false
            self.sessionId = nil
        }
    }

    // MARK: - Reading

    private func startReading() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            guard let self = self, let input = self.inputStream else { return }

            let bufferSize = 8192
            var buffer = [UInt8](repeating: 0, count: bufferSize)

            while input.hasBytesAvailable || input.streamStatus == .open {
                let bytesRead = input.read(&buffer, maxLength: bufferSize)
                if bytesRead > 0 {
                    self.readBuffer.append(contentsOf: buffer[0..<bytesRead])
                    self.processReadBuffer()
                } else if bytesRead < 0 {
                    self.logger.error("read error")
                    break
                } else {
                    // 0 bytes = would block, wait a bit
                    Thread.sleep(forTimeInterval: 0.01)
                }
            }

            DispatchQueue.main.async {
                self.isConnected = false
                self.scheduleReconnect()
            }
        }
    }

    private func processReadBuffer() {
        while let newlineIndex = readBuffer.firstIndex(of: UInt8(ascii: "\n")) {
            let lineData = readBuffer[readBuffer.startIndex..<newlineIndex]
            readBuffer = Data(readBuffer[(newlineIndex + 1)...])

            guard let json = try? JSONSerialization.jsonObject(with: lineData) as? [String: Any] else {
                continue
            }

            if let id = json["id"] as? Int {
                // Response to a request
                if let callback = pendingRequests.removeValue(forKey: id) {
                    if let result = json["result"] {
                        callback(.success(result))
                    } else if let error = json["error"] as? [String: Any] {
                        let msg = error["message"] as? String ?? "unknown error"
                        callback(.failure(NSError(domain: "SidecarBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: msg])))
                    }
                }
            } else if let method = json["method"] as? String {
                // Server notification
                let params = json["params"] as? [String: Any] ?? [:]
                DispatchQueue.main.async {
                    self.onNotification?(method, params)
                }
            }
        }
    }

    // MARK: - Sending

    func send(method: String, params: [String: Any] = [:], completion: ((Result<Any, Error>) -> Void)? = nil) {
        let id = nextRequestId
        nextRequestId += 1

        let request: [String: Any] = [
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        ]

        if let completion = completion {
            pendingRequests[id] = completion
        }

        guard let data = try? JSONSerialization.data(withJSONObject: request),
              let output = outputStream else {
            completion?(.failure(NSError(domain: "SidecarBridge", code: -1, userInfo: [NSLocalizedDescriptionKey: "not connected"])))
            return
        }

        var toSend = data
        toSend.append(UInt8(ascii: "\n"))
        toSend.withUnsafeBytes { ptr in
            _ = output.write(ptr.bindMemory(to: UInt8.self).baseAddress!, maxLength: toSend.count)
        }
    }

    /// Async wrapper for send
    @MainActor
    func send(method: String, params: [String: Any] = [:]) async throws -> Any {
        try await withCheckedThrowingContinuation { continuation in
            send(method: method, params: params) { result in
                continuation.resume(with: result)
            }
        }
    }

    private func sendHello() {
        send(method: "session.hello") { [weak self] result in
            guard let self = self else { return }
            if case .success(let value) = result,
               let dict = value as? [String: Any],
               let sid = dict["session_id"] as? String {
                DispatchQueue.main.async {
                    self.sessionId = sid
                    self.logger.info("session established: \(sid)")
                }
            }
        }
    }

    // MARK: - Helpers

    private func shell(_ command: String) -> String {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/bin/sh")
        proc.arguments = ["-c", command]
        let pipe = Pipe()
        proc.standardOutput = pipe
        proc.standardError = FileHandle.nullDevice
        try? proc.run()
        proc.waitUntilExit()
        return String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
    }
}

// MARK: - sockaddr_un helper

private extension sockaddr_un {
    static func make(path: String) -> sockaddr_un {
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = path.utf8CString
        withUnsafeMutablePointer(to: &addr.sun_path) { ptr in
            let bound = ptr.withMemoryRebound(to: Int8.self, capacity: Int(104)) { $0 }
            for (i, byte) in pathBytes.enumerated() where i < 104 {
                bound[i] = byte
            }
        }
        return addr
    }
}
