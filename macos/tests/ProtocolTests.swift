import Foundation

@main struct ProtocolTests {
    static func main() throws {
        precondition(MetricFormat.percent(nil) == "—")
        precondition(MetricFormat.percent(25.4) == "25%")
        precondition(MetricFormat.temperature(80) == "80°C")
        precondition(MetricFormat.rate(2048) == "2.0 KiB/s")
        precondition(MetricFormat.rate(UInt64.max) == "16 EiB/s")
        let json = #"{"v":1,"cpu":{"usage":null,"temp":null},"memory":{"used":50,"swap":null},"gpu":{"usage":null,"memory":null,"temp":null},"disk":{"read":0,"write":0,"temp":null},"net":{"rx":1024,"tx":0},"compatible":true}"#
        let sample = try JSONDecoder().decode(MetricSample.self, from: Data(json.utf8))
        precondition(sample.v == 1 && sample.cpu.usage == nil && sample.net.rx == 1024)
        for invalid in [json.replacingOccurrences(of: "\"usage\":null", with: "\"usage\":101"),
                        json.replacingOccurrences(of: "\"temp\":null", with: "\"other\":null"),
                        json.replacingOccurrences(of: "\"rx\":1024", with: "\"rx\":-1"),
                        json.replacingOccurrences(of: "\"v\":1", with: "\"v\":2")] {
            precondition((try? JSONDecoder().decode(MetricSample.self, from: Data(invalid.utf8))) == nil)
        }
        let resources = try UIResource.load(CommandLine.arguments[1])
        for code in ["en", "ru", "kk", "ar", "zh"] {
            precondition(resources.language(code + "-XX") == code)
            precondition(resources.text("cpu", language: code) != "cpu")
        }
        precondition(resources.language("system", preferred: ["fr", "kk-KZ"]) == "kk")
        precondition(resources.defaults.sampling.intervalSeconds == 5)
        precondition(!resources.defaults.sampling.gpu)
        print("PASS: typed v1 samples, unavailable metrics, finite formatting, binary rates")
    }
}
