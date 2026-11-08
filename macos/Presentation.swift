import Foundation

struct AppConfig: Decodable, Sendable, Equatable {
    struct Sampling: Decodable, Sendable, Equatable { let intervalSeconds: Int; let gpu: Bool }
    struct Transport: Decodable, Sendable, Equatable { let clientTimeoutMs: Int; let reconnectSeconds: Int }
    struct Display: Decodable, Sendable, Equatable {
        let locale: String; let theme: String; let density: String
        let showCpu: Bool; let showMemory: Bool; let showGpu: Bool
        let showNetwork: Bool; let showTemperatures: Bool
    }
    let sampling: Sampling
    let transport: Transport
    let display: Display
}

struct UIResource: Decodable, Sendable {
    struct Colors: Decodable, Sendable {
        let surface: String; let raised: String; let text: String
        let secondary: String; let border: String; let accent: String; let focus: String
    }
    struct Metrics: Decodable, Sendable {
        let gap: Double; let small: Double; let padding: Double; let radius: Double
        let brand: Double; let icon: Double; let minWidth: Double; let control: Double; let border: Double
    }
    struct Brand: Decodable, Sendable { let viewBox: [Double]; let paths: [String] }
    struct Typography: Decodable, Sendable { let body: Double; let label: Double; let card:Double }
    struct Series:Decodable,Sendable {let key:String;let label:String;let unit:String}
    struct Analysis:Decodable,Sendable {let frameBytes:Int;let plotColumns:Int;let historySeconds:Int;let rangesSeconds:[Int];let coresPerPage:Int;let viewVersion:Int}
    struct Cell:Decodable,Sendable {let label:String;let metric:String}
    let website:String
    let series:[Series]
    let gpuSeries:[Series]
    let analysis:Analysis
    let cells:[Cell]
    let version: String
    let catalogs: [String: [String: String]]
    let themes: [String: Colors]
    let metrics: [String: Metrics]
    let typography: [String: Typography]
    let brand: Brand
    let defaults: AppConfig

    static func load(_ path: String) throws -> UIResource {
        try decode(UIResource.self, Data(contentsOf: URL(fileURLWithPath: path)))
    }
    func language(_ requested: String, preferred: [String] = Locale.preferredLanguages) -> String {
        for value in requested == "system" ? preferred : [requested] {
            let code = value.lowercased().split(whereSeparator: { $0 == "-" || $0 == "_" }).first.map(String.init) ?? "en"
            if catalogs[code] != nil { return code }
        }
        return "en"
    }
    func text(_ key: String, language: String) -> String { catalogs[language]?[key] ?? catalogs["en"]?[key] ?? key }
    func interval(_ value: Int, language: String) -> String {
        let key = [0: "realtime", 2: "fast", 5: "standard", 10: "economy"][value]
        return key.map { text($0, language: language) }
            ?? text("custom_interval", language: language).replacingOccurrences(of: "{seconds}", with: String(value))
    }
}

func decode<T: Decodable>(_ type: T.Type, _ data: Data) throws -> T {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return try decoder.decode(type, from: data)
}

/// The Rust CLI owns configuration parsing, validation and atomic writes. Each
/// invocation ends when the operation completes; no extra resident helper.
struct ConfigCLI: Sendable {
    let executable: String
    let path: String?
    func run(_ arguments: [String]) async throws -> Data {
        let executable = self.executable
        let arguments = (path.map { ["--config", $0] } ?? []) + arguments
        return try await Task.detached(priority: .utility) {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: executable)
            process.arguments = arguments
            let output = Pipe()
            process.standardOutput = output
            // Parser details stay in the CLI's local diagnostics, not in UI text.
            process.standardError = FileHandle.nullDevice
            try process.run()
            let timeout = DispatchWorkItem { if process.isRunning { process.terminate() } }
            DispatchQueue.global(qos: .utility).asyncAfter(deadline: .now() + 5, execute: timeout)
            let data = output.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            timeout.cancel()
            guard process.terminationStatus == 0 else {
                throw CocoaError(.executableRuntimeMismatch)
            }
            return data
        }.value
    }
    func load() async throws -> AppConfig { try decode(AppConfig.self, await run(["config", "show", "--json"])) }
}

enum MetricFormat {
    static let absent = "—"
    static func percent(_ value: Double?) -> String {
        value.map { String(format: "%.0f%%", $0) } ?? absent
    }
    static func temperature(_ value: Double?) -> String {
        value.map { String(format: "%.0f°C", $0) } ?? absent
    }
    static func rate(_ value: UInt64?) -> String {
        binary(value.map(Double.init), rate: true)
    }
    static func bytes(_ value: Double?) -> String { binary(value, rate: false) }
    private static func binary(_ value: Double?, rate: Bool) -> String {
        guard let value, value.isFinite, value >= 0 else { return absent }
        var amount = value
        let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"]
        var unit = 0
        while amount >= 1024 && unit < units.count - 1 { amount /= 1024; unit += 1 }
        return "\(String(format: "%.*f", unit > 0 && amount < 10 ? 1 : 0, amount)) \(units[unit])\(rate ? "/s" : "")"
    }
    static func readings(_ sample: MetricSample?) -> [(String, String)] {
        [("cpu", percent(sample?.cpu.usage)), ("cpu_temperature", temperature(sample?.cpu.temp)),
         ("memory", percent(sample?.memory.used)), ("swap", percent(sample?.memory.swap)),
         ("gpu", percent(sample?.gpu.usage)), ("gpu_memory", percent(sample?.gpu.memory)),
         ("gpu_temperature", temperature(sample?.gpu.temp)), ("disk_read", rate(sample?.disk.read)),
         ("disk_write", rate(sample?.disk.write)), ("disk_temperature", temperature(sample?.disk.temp)),
         ("network_rx", rate(sample?.net.rx)), ("network_tx", rate(sample?.net.tx))]
    }
    static func isolate(_ value: String) -> String { "\u{2068}\(value)\u{2069}" }
}
