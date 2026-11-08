import AppKit
import Foundation

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    private let statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private let menu = NSMenu()
    private var rows: [String: NSMenuItem] = [:]
    private var stateItem = NSMenuItem()
    private var copyItem = NSMenuItem()
    private var settingsItem = NSMenuItem()
    private var client: MetricsClient?
    private var sample: MetricSample?
    private var resources: UIResource?
    private var config: AppConfig?
    private var cli: ConfigCLI!
    private var language = "en"
    private var clientGeneration: UInt64 = 0
    private var loading = false
    private var saving = false
    private var loadFailed = false
    private var watcher: DispatchSourceFileSystemObject?
    private var watchedDirectory: URL?
    private var configFile: URL?
    private var localeObserver: NSObjectProtocol?

    func applicationDidFinishLaunching(_ notification: Notification) {
        let directory = Bundle.main.executableURL!.deletingLastPathComponent()
        cli = ConfigCLI(executable: option("--cli") ?? directory.appendingPathComponent("nddev-systeminfo").path,
                        path: option("--config"))
        do {
            resources = try UIResource.load(option("--resources")
                ?? Bundle.main.resourceURL!.appendingPathComponent("ui.json").path)
        } catch {
            // Missing/corrupt packaged resources is an installation failure.
            statusItem.button?.title = "NDDev Systeminfo !"
            statusItem.button?.setAccessibilityLabel("NDDev Systeminfo")
            NSApplication.shared.terminate(nil)
            return
        }
        language = resources!.language("system")
        statusItem.button?.font = NSFont.monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .medium)
        statusItem.button?.image = brandImage(resources!.brand, size: resources!.metrics["compact"]!.control)
        statusItem.button?.imagePosition = .imageLeading
        menu.delegate = self
        menu.autoenablesItems = false
        statusItem.menu = menu
        rebuildMenu()
        localeObserver = NotificationCenter.default.addObserver(
            forName: NSLocale.currentLocaleDidChangeNotification, object: nil, queue: .main
        ) { [weak self] _ in Task { @MainActor in self?.rebuildMenu() } }
        Task {
            await initializeConfig()
            #if UI_ACCEPTANCE
            do { try await runNativeAcceptance(self) }
            catch { fputs("native acceptance failed: \(error)\n", stderr); exit(1) }
            #endif
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        clientGeneration &+= 1
        client?.stop()
        client = nil
        watcher?.cancel()
        watcher = nil
        if let localeObserver { NotificationCenter.default.removeObserver(localeObserver) }
    }
    func menuWillOpen(_ menu: NSMenu) { Task { await reload() } }

    private func initializeConfig() async {
        do {
            let data = try await cli.run(["config", "path"])
            let path = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
            configFile = URL(fileURLWithPath: path)
            if !FileManager.default.fileExists(atPath: path) {
                // Read only explicit historical values. Preserve interval=0 and
                // false; an existing TOML file always wins in the Rust CLI.
                if let interval = UserDefaults.standard.persistentDomain(forName: "com.nddev-opennetwork.rldyour-sysinfo")?["interval"] as? Int {
                    let legacy = try JSONSerialization.data(withJSONObject: ["interval": min(max(interval, 0), WireContract.maxInterval)])
                    _ = try await cli.run(["config", "migrate", "--legacy-json", String(decoding: legacy, as: UTF8.self)])
                }
            }
            installWatcher()
        } catch { /* reload presents a localized error and can recover. */ }
        await reload()
    }
    private func reload() async {
        guard !loading, !saving else { return }
        loading = true
        defer { loading = false }
        do {
            let new = try await cli.load()
            let changed = config != new
            let recovered = loadFailed
            loadFailed = false
            config = new
            if changed || recovered {
                rebuildMenu()
                restartClient()
            }
            installWatcher()
        } catch {
            loadFailed = true
            settingsItem.isEnabled = false
            stateItem.title = text("error")
            if config == nil { update(nil) }
        }
    }
    private func installWatcher() {
        guard var directory = configFile?.deletingLastPathComponent() else { return }
        while !FileManager.default.fileExists(atPath: directory.path) && directory.path != "/" {
            directory.deleteLastPathComponent()
        }
        guard directory != watchedDirectory else { return }
        watcher?.cancel()
        watcher = nil
        let fd = open(directory.path, O_EVTONLY | O_CLOEXEC)
        guard fd >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(fileDescriptor: fd,
            eventMask: [.write, .rename, .delete], queue: .main)
        source.setEventHandler { [weak self] in Task { @MainActor in await self?.reload() } }
        source.setCancelHandler { Darwin.close(fd) }
        watchedDirectory = directory
        watcher = source
        source.resume()
    }
    private func restartClient() {
        clientGeneration &+= 1
        client?.stop()
        client = nil
        update(nil)
        guard let config else { return }
        let current = clientGeneration
        client = MetricsClient(interval: config.sampling.intervalSeconds,
            timeoutMs: config.transport.clientTimeoutMs, reconnectSeconds: config.transport.reconnectSeconds) { [weak self] event in
            Task { @MainActor in
                guard let self, self.clientGeneration == current else { return }
                switch event {
                case .sample(let value): self.update(value)
                case .offline: self.update(nil)
                }
            }
        }
        client?.start()
    }
    private func text(_ key: String) -> String { resources!.text(key, language: language) }
    private func action(_ key: String, _ selector: Selector) -> NSMenuItem {
        let item = NSMenuItem(title: text(key), action: selector, keyEquivalent: "")
        item.target = self
        return item
    }
    private func rebuildMenu() {
        guard let resources else { return }
        language = resources.language(config?.display.locale ?? "system")
        menu.removeAllItems()
        rows.removeAll()
        menu.userInterfaceLayoutDirection = language == "ar" ? .rightToLeft : .leftToRight
        let header = NSMenuItem()
        header.view = BrandHeader(resource: resources, config: config ?? resources.defaults,
                                  language: language)
        menu.addItem(header)
        stateItem = NSMenuItem(title: text(config == nil ? "loading" : "disconnected"), action: nil, keyEquivalent: "")
        stateItem.isEnabled = false
        menu.addItem(stateItem)
        for (key, _) in MetricFormat.readings(nil) {
            let item = NSMenuItem(title: "", action: nil, keyEquivalent: "")
            rows[key] = item
            menu.addItem(item)
        }
        menu.addItem(.separator())
        copyItem = action("copy_report", #selector(copyReport))
        menu.addItem(copyItem)
        menu.addItem(action("retry", #selector(reconnect)))
        settingsItem = NSMenuItem(title: text("settings"), action: nil, keyEquivalent: "")
        settingsItem.isEnabled = config != nil && !saving && !loadFailed
        settingsItem.submenu = settingsMenu()
        menu.addItem(settingsItem)
        menu.addItem(.separator())
        let quit = action("quit", #selector(quitApp))
        quit.keyEquivalent = "q"
        menu.addItem(quit)
        update(sample)
    }
    private func settingsMenu() -> NSMenu {
        let result = NSMenu()
        result.autoenablesItems = false
        result.userInterfaceLayoutDirection = menu.userInterfaceLayoutDirection
        guard let config else { return result }
        func submenu(_ key: String) -> NSMenu {
            let parent = NSMenuItem(title: text(key), action: nil, keyEquivalent: "")
            let child = NSMenu()
            child.autoenablesItems = false
            child.userInterfaceLayoutDirection = result.userInterfaceLayoutDirection
            parent.submenu = child
            result.addItem(parent)
            return child
        }
        func setting(_ parent: NSMenu, _ title: String, _ key: String, _ value: String, _ selected: Bool) {
            let item = NSMenuItem(title: title, action: #selector(changeSetting(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = [key, value]
            item.state = selected ? .on : .off
            parent.addItem(item)
        }
        let interval = submenu("interval")
        let values = Array(Set([0, 2, 5, 10, config.sampling.intervalSeconds])).sorted()
        for value in values {
            setting(interval, resources!.interval(value, language: language), "sampling.interval_seconds", String(value), value == config.sampling.intervalSeconds)
        }
        setting(result, text("collect_gpu"), "sampling.gpu", String(!config.sampling.gpu), config.sampling.gpu)
        let panel = submenu("panel")
        for (key, selected) in [("show_cpu", config.display.showCpu), ("show_memory", config.display.showMemory),
            ("show_gpu", config.display.showGpu), ("show_network", config.display.showNetwork),
            ("show_temperatures", config.display.showTemperatures)] {
            setting(panel, text(key), "display.\(key)", String(!selected), selected)
        }
        let themes = submenu("theme")
        for value in ["system", "light", "dark", "light-contrast", "dark-contrast"] {
            setting(themes, text(value), "display.theme", value, config.display.theme == value)
        }
        let density = submenu("density")
        for value in ["compact", "comfortable"] {
            setting(density, text(value), "display.density", value, config.display.density == value)
        }
        let languages = submenu("language")
        for (code, title) in [("system", text("system")), ("en", "English"), ("ru", "Русский"),
                              ("kk", "Қазақша"), ("ar", "العربية"), ("zh", "中文")] {
            setting(languages, title, "display.locale", code, config.display.locale == code)
        }
        return result
    }
    private func update(_ value: MetricSample?) {
        sample = value
        for (key, reading) in MetricFormat.readings(value) {
            let title = "\(text(key)): \(MetricFormat.isolate(reading))"
            if rows[key]?.title != title { rows[key]?.title = title }
        }
        copyItem.isEnabled = value != nil
        stateItem.title = text(loadFailed ? "error" : config == nil ? "loading" : value == nil ? "disconnected" : "connected")
        guard let display = config?.display else {
            statusItem.button?.title = " …"
            return
        }
        var parts: [String] = []
        if display.showCpu { parts.append("CPU \(MetricFormat.percent(value?.cpu.usage))") }
        if display.showMemory { parts.append("RAM \(MetricFormat.percent(value?.memory.used))") }
        if display.showGpu { parts.append("GPU \(MetricFormat.percent(value?.gpu.usage))") }
        if display.showNetwork { parts.append("↓\(MetricFormat.rate(value?.net.rx)) ↑\(MetricFormat.rate(value?.net.tx))") }
        if display.showTemperatures { parts.append(MetricFormat.temperature(value?.cpu.temp)) }
        let title = parts.map(MetricFormat.isolate).joined(separator: "  ")
        if statusItem.button?.title != title { statusItem.button?.title = title }
        statusItem.button?.setAccessibilityLabel("\(text("app_title")), \(text(value == nil ? "disconnected" : "connected")), \(parts.joined(separator: ", "))")
    }
    @objc private func changeSetting(_ sender: NSMenuItem) {
        guard !saving, let pair = sender.representedObject as? [String], pair.count == 2 else { return }
        saving = true
        settingsItem.isEnabled = false
        Task {
            do {
                _ = try await cli.run(["config", "set", pair[0], pair[1], "--json"])
                saving = false
                await reload()
                if pair[0].hasPrefix("sampling.") { stateItem.title = text("restart_provider") }
            } catch {
                saving = false
                settingsItem.isEnabled = true
                stateItem.title = text("error")
            }
        }
    }
    @objc private func copyReport() {
        guard let sample, let data = try? JSONEncoder().encode(sample) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(String(decoding: data, as: UTF8.self), forType: .string)
    }
    @objc private func reconnect() { restartClient() }
    @objc private func quitApp() { NSApplication.shared.terminate(nil) }
    #if UI_ACCEPTANCE
    var acceptanceMenu: NSMenu { menu }
    var acceptanceConfig: AppConfig? { config }
    var acceptanceHasSample: Bool { sample != nil }
    var acceptanceCLI: ConfigCLI { cli }
    func acceptanceReload() async { await reload() }
    #endif
}

private func option(_ name: String) -> String? {
    guard let index = CommandLine.arguments.firstIndex(of: name), index + 1 < CommandLine.arguments.count else { return nil }
    return CommandLine.arguments[index + 1]
}

@main
struct SysinfoApp {
    @MainActor static func main() {
        let app = NSApplication.shared
        if let identifier = Bundle.main.bundleIdentifier,
           NSRunningApplication.runningApplications(withBundleIdentifier: identifier)
               .contains(where: { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }) { return }
        let delegate = AppDelegate()
        app.delegate = delegate
        app.setActivationPolicy(.accessory)
        app.run()
    }
}
