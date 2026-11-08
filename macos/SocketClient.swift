import Foundation

/// Every descriptor, source and buffer is confined to this serial queue. Values
/// cross to AppKit through a Sendable callback; cancellation never blocks it.
final class MetricsClient: @unchecked Sendable {
    enum Event: Sendable { case sample(MetricSample); case offline }
    private let queue = DispatchQueue(label: "org.nddev.systeminfo.socket", qos: .utility)
    private let onEvent: @Sendable (Event) -> Void
    private let interval: Int
    private let path: String
    private let timeout: Double
    private let retrySeconds: Double
    private var reader: DispatchSourceRead?
    private var writer: DispatchSourceWrite?
    private var readerResumed = false
    private var retry: DispatchWorkItem?
    private var deadline: DispatchWorkItem?
    private var generation: UInt64 = 0
    private var bytes = [UInt8](repeating: 0, count: WireContract.maxFrame)
    private var buffer = Data()
    private let announcement: Data
    private var written = 0
    private let decoder = JSONDecoder()
    private var stopped = false

    init(interval: Int, timeoutMs: Int = 5000, reconnectSeconds: Int = 5,
         path: String = FileManager.default.homeDirectoryForCurrentUser
             .appendingPathComponent("Library/Application Support/rldyour-sysinfo/rldyour-sysinfo.sock").path,
         onEvent: @escaping @Sendable (Event) -> Void) {
        self.interval = min(max(interval, 0), WireContract.maxInterval)
        self.path = path
        self.timeout = Double(timeoutMs) / 1000
        self.retrySeconds = Double(reconnectSeconds)
        self.onEvent = onEvent
        self.announcement = Data("{\"interval\":\(self.interval),\"record\":true}\n".utf8)
        buffer.reserveCapacity(WireContract.maxFrame)
    }

    func start() { queue.async { self.connect() } }
    func stop() {
        queue.async {
            self.stopped = true
            self.retry?.cancel()
            self.retry = nil
            self.close()
        }
    }

    private func connect() {
        guard !stopped, reader == nil else { return }
        retry = nil
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { return reconnect() }
        var ownsFD = true
        defer { if ownsFD { Darwin.close(fd) } }
        var noSigPipe: Int32 = 1
        guard setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, &noSigPipe, socklen_t(MemoryLayout<Int32>.size)) == 0,
              fcntl(fd, F_SETFD, FD_CLOEXEC) == 0,
              fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK) == 0 else { return reconnect() }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        guard path.utf8.count < MemoryLayout.size(ofValue: address.sun_path) else { return reconnect() }
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.initializeMemory(as: UInt8.self, repeating: 0)
            for (index, byte) in path.utf8.enumerated() { raw[index] = byte }
        }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 || errno == EINPROGRESS else { return reconnect() }
        let current = generation
        let readSource = DispatchSource.makeReadSource(fileDescriptor: fd, queue: queue)
        readSource.setEventHandler { [weak self] in
            guard let self, self.generation == current else { return }
            self.read(fd)
        }
        // The read source alone owns the descriptor, even during connection.
        readSource.setCancelHandler { Darwin.close(fd) }
        reader = readSource
        ownsFD = false
        let writeSource = DispatchSource.makeWriteSource(fileDescriptor: fd, queue: queue)
        writeSource.setEventHandler { [weak self] in
            guard let self, self.generation == current else { return }
            self.handshake(fd)
        }
        writer = writeSource
        armDeadline(seconds: timeout)
        writeSource.resume()
    }

    private func handshake(_ fd: Int32) {
        var error: Int32 = 0
        var length = socklen_t(MemoryLayout<Int32>.size)
        guard getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &length) == 0, error == 0 else { return reconnect() }
        while written < announcement.count {
            let count = announcement.withUnsafeBytes {
                Darwin.write(fd, $0.baseAddress!.advanced(by: written), $0.count - written)
            }
            if count < 0 {
                if errno == EINTR { continue }
                if errno == EAGAIN || errno == EWOULDBLOCK { return }
                return reconnect()
            }
            guard count > 0 else { return reconnect() }
            written += count
        }
        writer?.cancel()
        writer = nil
        readerResumed = true
        reader?.resume()
        armFrameDeadline()
    }

    private func read(_ fd: Int32) {
        while true {
            let count = Darwin.read(fd, &bytes, bytes.count)
            if count < 0 {
                if errno == EINTR { continue }
                if errno == EAGAIN || errno == EWOULDBLOCK { return }
                return reconnect()
            }
            if count == 0 { return reconnect() }
            for byte in bytes.prefix(count) {
                if byte == 10 {
                    guard let sample = try? decoder.decode(MetricSample.self, from: buffer) else { return reconnect() }
                    buffer.removeAll(keepingCapacity: true)
                    onEvent(.sample(sample))
                    armFrameDeadline()
                } else {
                    guard buffer.count < WireContract.maxFrame else { return reconnect() }
                    buffer.append(byte)
                }
            }
        }
    }

    private func armFrameDeadline() {
        // A frame has one absolute deadline. Partial/trickled bytes never extend it.
        let cadence = interval == 0 ? Double(WireContract.realtimeMs) / 1000 : Double(interval)
        armDeadline(seconds: cadence + timeout)
    }
    private func armDeadline(seconds: Double) {
        deadline?.cancel()
        let current = generation
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.generation == current, !self.stopped else { return }
            self.reconnect()
        }
        deadline = work
        queue.asyncAfter(deadline: .now() + seconds, execute: work)
    }
    private func close() {
        generation &+= 1
        deadline?.cancel()
        deadline = nil
        writer?.cancel()
        writer = nil
        reader?.cancel()
        if !readerResumed { reader?.resume() }
        reader = nil
        readerResumed = false
        written = 0
        buffer.removeAll(keepingCapacity: true)
    }
    private func reconnect() {
        close()
        guard !stopped else { return }
        onEvent(.offline)
        retry?.cancel()
        let work = DispatchWorkItem { [weak self] in self?.connect() }
        retry = work
        queue.asyncAfter(deadline: .now() + retrySeconds, execute: work)
    }
}
