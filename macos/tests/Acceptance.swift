import AppKit
import Foundation

/// Compiled into the real application only for native CI acceptance. Actions
/// traverse the same NSMenuItem targets and Rust CLI as a user's selection.
@MainActor
func runNativeAcceptance(_ app: AppDelegate) async throws {
    let arguments = CommandLine.arguments
    let folder = URL(fileURLWithPath: arguments[arguments.firstIndex(of: "--artifacts")! + 1])
    try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
    try await until { app.acceptanceHasSample }
    precondition(app.acceptanceMenu.items.count >= 19)
    let cli = app.acceptanceCLI

    func select(_ key: String, _ value: String) async throws {
        func find(_ menu: NSMenu) -> (NSMenu, Int)? {
            for (index, item) in menu.items.enumerated() {
                if item.representedObject as? [String] == [key, value] { return (menu, index) }
                if let child = item.submenu, let match = find(child) { return match }
            }
            return nil
        }
        guard let (menu, index) = find(app.acceptanceMenu) else { throw TestFailure.missingSetting }
        menu.performActionForItem(at: index)
        try await until {
            let config = app.acceptanceConfig
            switch key {
            case "sampling.interval_seconds": return config?.sampling.intervalSeconds == Int(value)
            case "display.locale": return config?.display.locale == value
            case "display.theme": return config?.display.theme == value
            default: return false
            }
        }
        let persisted = try await cli.load()
        precondition(persisted == app.acceptanceConfig)
    }
    for (code, theme) in [("en", "light"), ("ru", "dark"), ("kk", "light-contrast"),
                           ("ar", "dark-contrast"), ("zh", "light")] {
        if app.acceptanceConfig?.display.locale != code { try await select("display.locale", code) }
        if app.acceptanceConfig?.display.theme != theme { try await select("display.theme", theme) }
        try await until { app.acceptanceHasSample }
        precondition(app.acceptanceMenu.userInterfaceLayoutDirection == (code == "ar" ? .rightToLeft : .leftToRight))
        try captureMenu(app.acceptanceMenu, folder.appendingPathComponent("\(code)-\(theme).png"))
        app.acceptanceOpenAnalysis()
        try await until { app.acceptanceAnalysis?.acceptanceLive == true && app.acceptanceAnalysis?.acceptanceView?.metric == "cpu.usage"
            && app.acceptanceAnalysis?.acceptanceView?.statistics.current != nil }
        guard let analysis = app.acceptanceAnalysis, let window = analysis.window else { throw TestFailure.missingSetting }
        window.displayIfNeeded()
        try captureWindow(window, folder.appendingPathComponent("analysis-\(code)-\(theme).png"))
    }
    guard let analysis = app.acceptanceAnalysis,
          let core = analysis.acceptanceView?.latest?.detail.cores.first else { throw TestFailure.missingSetting }
    analysis.acceptanceSelect("core:\(core.id)")
    try await until { analysis.acceptanceView?.metric == "core:\(core.id)" }
    analysis.acceptancePause()
    precondition(analysis.acceptancePaused && !analysis.acceptanceHasReader)
    let pausedSequence = analysis.acceptanceView?.latest?.detail.sequence
    try await Task.sleep(nanoseconds: 600_000_000)
    precondition(analysis.acceptanceView?.latest?.detail.sequence == pausedSequence)
    analysis.acceptancePause()
    try await until { !analysis.acceptancePaused && analysis.acceptanceHasReader }
    try captureWindow(analysis.window!, folder.appendingPathComponent("logical-core.png"))
    analysis.close()
    precondition(app.acceptanceAnalysis == nil)
    try await select("sampling.interval_seconds", "0")
    try await until { app.acceptanceHasSample }
    guard let copyIndex = app.acceptanceMenu.items.firstIndex(where: { $0.action == NSSelectorFromString("copyReport") }) else { throw TestFailure.missingSetting }
    app.acceptanceMenu.performActionForItem(at: copyIndex)
    let copied = NSPasteboard.general.string(forType: .string)!
    _ = try JSONDecoder().decode(MetricSample.self, from: Data(copied.utf8))
    let provider = Int32(arguments[arguments.firstIndex(of: "--provider-pid")! + 1])!
    precondition(kill(provider, SIGTERM) == 0)
    try await until { !app.acceptanceHasSample }
    precondition(!app.acceptanceMenu.items[copyIndex].isEnabled)
    let rows = app.acceptanceMenu.items.filter { $0.title.contains("\u{2068}—\u{2069}") }
    precondition(rows.count == 12)
    try captureMenu(app.acceptanceMenu, folder.appendingPathComponent("offline.png"))
    try Data("{\"native_menu\":true,\"analysis_window\":true,\"logical_core\":true,\"pause_resume_close\":true,\"locales\":5,\"themes\":4,\"settings\":true,\"clipboard\":true,\"offline_cleared\":12}\n".utf8)
        .write(to: folder.appendingPathComponent("result.json"), options: .atomic)
    print("PASS: real AppKit menu, five locales, four themes, settings, clipboard and provider disconnect")
    NSApplication.shared.terminate(nil)
}

private enum TestFailure: Error { case timeout, missingSetting, screenshot }
@MainActor
private func captureWindow(_ window: NSWindow, _ path: URL) throws {
    window.displayIfNeeded()
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
    process.arguments = ["-x", "-l", String(window.windowNumber), path.path]
    try process.run(); process.waitUntilExit()
    guard process.terminationStatus == 0 && NSImage(contentsOf: path) != nil else { throw TestFailure.screenshot }
}
@MainActor
private func until(_ predicate: () -> Bool) async throws {
    let deadline = ProcessInfo.processInfo.systemUptime + 15
    while !predicate() {
        guard ProcessInfo.processInfo.systemUptime < deadline else { throw TestFailure.timeout }
        try await Task.sleep(nanoseconds: 20_000_000)
    }
}
@MainActor
private func captureMenu(_ menu: NSMenu, _ path: URL) throws {
    // Native tracking lays out and paints the actual menu. A queued capture runs
    // in its tracking mode, then cancels tracking so the scenario can continue.
    let capture = MenuCapture(menu: menu, path: path)
    capture.perform(#selector(MenuCapture.capture), with: nil, afterDelay: 0, inModes: [.eventTracking])
    menu.popUp(positioning: nil, at: NSPoint(x: 100, y: NSScreen.main!.visibleFrame.maxY - 30), in: nil)
    guard capture.captured else { throw TestFailure.screenshot }
}

@MainActor
private final class MenuCapture: NSObject {
    let menu: NSMenu
    let path: URL
    var captured = false
    let deadline = ProcessInfo.processInfo.systemUptime + 5
    init(menu: NSMenu, path: URL) { self.menu = menu; self.path = path }
    @objc func capture() {
        let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
        let window = windows.first {
            guard ($0[kCGWindowOwnerPID as String] as? Int) == Int(getpid()),
                  let bounds = $0[kCGWindowBounds as String] as? [String: Double] else { return false }
            return (bounds["Height"] ?? 0) > 100 && (bounds["Width"] ?? 0) > 100
        }
        guard let number = window?[kCGWindowNumber as String] as? UInt32 else {
            if ProcessInfo.processInfo.systemUptime < deadline {
                perform(#selector(capture), with: nil, afterDelay: 0.02, inModes: [.eventTracking])
            } else { menu.cancelTracking() }
            return
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        process.arguments = ["-x", "-l", String(number), path.path]
        do {
            try process.run()
            process.waitUntilExit()
            captured = process.terminationStatus == 0 && NSImage(contentsOf: path) != nil
        } catch { captured = false }
        menu.cancelTracking()
    }
}
