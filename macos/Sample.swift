// Generated from contracts/snapshot-v1.schema.json; do not edit.
import Foundation

enum WireContract {
    static let version = 1
    static let maxFrame = 4096
    static let maxInterval = 60
    static let realtimeMs = 500
}

struct MetricSample: Codable, Sendable {
    struct CPU: Codable, Sendable {
        let usage: Double?
        let temp: Double?
        init(from decoder: any Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            usage = try values.decode(Double?.self, forKey: .usage)
            if let value = usage, !(value.isFinite && (0...100).contains(value)) {
                throw DecodingError.dataCorruptedError(forKey: .usage, in: values, debugDescription: "Invalid metric")
            }
            temp = try values.decode(Double?.self, forKey: .temp)
            if let value = temp, !(value.isFinite) {
                throw DecodingError.dataCorruptedError(forKey: .temp, in: values, debugDescription: "Invalid metric")
            }
        }
    }
    struct MEMORY: Codable, Sendable {
        let used: Double?
        let swap: Double?
        init(from decoder: any Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            used = try values.decode(Double?.self, forKey: .used)
            if let value = used, !(value.isFinite && (0...100).contains(value)) {
                throw DecodingError.dataCorruptedError(forKey: .used, in: values, debugDescription: "Invalid metric")
            }
            swap = try values.decode(Double?.self, forKey: .swap)
            if let value = swap, !(value.isFinite && (0...100).contains(value)) {
                throw DecodingError.dataCorruptedError(forKey: .swap, in: values, debugDescription: "Invalid metric")
            }
        }
    }
    struct GPU: Codable, Sendable {
        let usage: Double?
        let memory: Double?
        let temp: Double?
        init(from decoder: any Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            usage = try values.decode(Double?.self, forKey: .usage)
            if let value = usage, !(value.isFinite && (0...100).contains(value)) {
                throw DecodingError.dataCorruptedError(forKey: .usage, in: values, debugDescription: "Invalid metric")
            }
            memory = try values.decode(Double?.self, forKey: .memory)
            if let value = memory, !(value.isFinite && (0...100).contains(value)) {
                throw DecodingError.dataCorruptedError(forKey: .memory, in: values, debugDescription: "Invalid metric")
            }
            temp = try values.decode(Double?.self, forKey: .temp)
            if let value = temp, !(value.isFinite) {
                throw DecodingError.dataCorruptedError(forKey: .temp, in: values, debugDescription: "Invalid metric")
            }
        }
    }
    struct DISK: Codable, Sendable {
        let read: UInt64?
        let write: UInt64?
        let temp: Double?
        init(from decoder: any Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            read = try values.decode(UInt64?.self, forKey: .read)
            write = try values.decode(UInt64?.self, forKey: .write)
            temp = try values.decode(Double?.self, forKey: .temp)
            if let value = temp, !(value.isFinite) {
                throw DecodingError.dataCorruptedError(forKey: .temp, in: values, debugDescription: "Invalid metric")
            }
        }
    }
    struct NET: Codable, Sendable {
        let rx: UInt64?
        let tx: UInt64?
        init(from decoder: any Decoder) throws {
            let values = try decoder.container(keyedBy: CodingKeys.self)
            rx = try values.decode(UInt64?.self, forKey: .rx)
            tx = try values.decode(UInt64?.self, forKey: .tx)
        }
    }
    let v: Int
    let cpu: CPU
    let memory: MEMORY
    let gpu: GPU
    let disk: DISK
    let net: NET
    init(from decoder: any Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        v = try values.decode(Int.self, forKey: .v)
        guard v == WireContract.version else {
            throw DecodingError.dataCorruptedError(forKey: .v, in: values, debugDescription: "Unsupported protocol version")
        }
        cpu = try values.decode(CPU.self, forKey: .cpu)
        memory = try values.decode(MEMORY.self, forKey: .memory)
        gpu = try values.decode(GPU.self, forKey: .gpu)
        disk = try values.decode(DISK.self, forKey: .disk)
        net = try values.decode(NET.self, forKey: .net)
    }
}
