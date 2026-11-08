import AppKit
import Foundation

// Rust owns retention, gap detection, decimation and statistics. AppKit keeps
// only one bounded view, and draws only when that view or appearance changes.
struct AnalysisView: Decodable, Sendable {
    struct Core: Decodable, Sendable { let id: UInt32; let usage: Double? }
    struct GPU: Decodable, Sendable {
        let id: String; let name: String; let status: String
        let usage: Double?; let temperature: Double?
        let dedicatedUsed: UInt64?; let dedicatedTotal: UInt64?; let sharedUsed: UInt64?
    }
    struct Detail: Decodable, Sendable {
        let sequence: UInt64
        let gpuEnabled: Bool; let cores: [Core]; let gpus: [GPU]
    }
    struct Frame: Decodable, Sendable {
        let sample: MetricSample
        let detail: Detail
        private enum CodingKeys: CodingKey { case detail }
        init(from decoder: any Decoder) throws {
            sample = try MetricSample(from: decoder)
            detail = try decoder.container(keyedBy: CodingKeys.self).decode(Detail.self, forKey: .detail)
        }
    }
    struct Point: Decodable, Sendable { let xMs: Double; let low: Double; let high: Double; let value: Double; let gap: Bool }
    struct Statistics: Decodable, Sendable {
        let current: Double?; let minimum: Double?; let maximum: Double?; let average: Double?
        let samples: Int; let coveredMs: Double
    }
    let latest: Frame?
    let metric: String; let label: String; let unit: String
    let endMs: Double; let availableMs: Double; let rangeSeconds: Double
    let yMin: Double; let yMax: Double
    let points: [Point]; let statistics: Statistics
}

/// The queue exclusively owns the child, descriptor and byte buffer. stop()
/// cancels the descriptor and terminates/reaps this owned child off the UI thread.
final class AnalysisReader: @unchecked Sendable {
    enum Event: Sendable { case view(AnalysisView, Data); case offline }
    private let queue = DispatchQueue(label: "org.nddev.systeminfo.analysis", qos: .utility)
    private let callback: @Sendable (Event) -> Void
    private var process: Process?
    private var source: DispatchSourceRead?
    private var buffer = Data()
    private var stopped = false
    private let limit: Int
    init(cli: ConfigCLI, resource: UIResource, metric: String, range: Int,
         callback: @escaping @Sendable (Event) -> Void) {
        self.callback = callback
        limit = resource.analysis.frameBytes
        queue.async {
            guard !self.stopped else { return }
            let child = Process()
            let pipe = Pipe()
            child.executableURL = URL(fileURLWithPath: cli.executable)
            child.arguments = (cli.path.map { ["--config", $0] } ?? []) + ["analyze", "--metric", metric,
                "--range-seconds", String(range), "--columns", String(resource.analysis.plotColumns)]
            child.standardOutput = pipe
            child.standardError = FileHandle.nullDevice
            do { try child.run() } catch { self.callback(.offline); return }
            self.process = child
            let fd = pipe.fileHandleForReading.fileDescriptor
            guard fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK) == 0 else {
                self.finish(); return
            }
            let reader = DispatchSource.makeReadSource(fileDescriptor: fd, queue: self.queue)
            reader.setEventHandler { [weak self] in self?.read(fd) }
            reader.setCancelHandler { try? pipe.fileHandleForReading.close() }
            self.source = reader
            reader.resume()
        }
    }
    func stop() { queue.async { self.stopped = true; self.finish(notify: false) } }
    private func read(_ fd: Int32) {
        var bytes = [UInt8](repeating: 0, count: 8192)
        while !stopped {
            let count = Darwin.read(fd, &bytes, bytes.count)
            if count < 0 {
                if errno == EINTR { continue }
                if errno == EAGAIN || errno == EWOULDBLOCK { return }
                return finish()
            }
            guard count > 0 else { return finish() }
            for byte in bytes.prefix(count) {
                if byte == 10 {
                    guard let view = try? decode(AnalysisView.self, buffer) else { return finish() }
                    callback(.view(view, buffer))
                    buffer.removeAll(keepingCapacity: true)
                } else {
                    guard buffer.count < limit else { return finish() }
                    buffer.append(byte)
                }
            }
        }
    }
    private func finish(notify: Bool = true) {
        source?.cancel(); source = nil
        if let child = process {
            process = nil
            if child.isRunning { child.terminate() }
            DispatchQueue.global(qos: .utility).async { child.waitUntilExit() }
        }
        buffer = Data()
        if notify && !stopped { callback(.offline) }
    }
}

@MainActor
func uiColor(_ hex: String) -> NSColor {
    let value = UInt32(hex.dropFirst(), radix: 16) ?? 0
    return NSColor(srgbRed: Double((value >> 16) & 255) / 255,
        green: Double((value >> 8) & 255) / 255, blue: Double(value & 255) / 255, alpha: 1)
}

@MainActor
private final class MetricCell: NSButton {
    var colors: UIResource.Colors!
    var metrics: UIResource.Metrics!
    var type: UIResource.Typography!
    var caption = ""
    var reading = ""
    var selected = false
    var choose: (() -> Void)?
    init(caption: String, reading: String, resource: UIResource, config: AppConfig, language: String,
         choose: @escaping () -> Void) {
        self.caption = caption; self.reading = reading; self.choose = choose
        metrics = resource.metrics[config.display.density]!
        type = resource.typography[language]!
        super.init(frame: .zero)
        setButtonType(.momentaryPushIn)
        isBordered = false
        target = self; action = #selector(activate)
        translatesAutoresizingMaskIntoConstraints = false
        heightAnchor.constraint(equalToConstant: 2 * metrics.padding + type.label + type.card + metrics.small).isActive = true
        update(reading)
    }
    required init?(coder: NSCoder) { nil }
    func update(_ value: String) {
        reading = value
        title = "\(caption): \(value)"
        setAccessibilityLabel(title)
        needsDisplay = true
    }
    @objc private func activate() { choose?() }
    override func draw(_ dirtyRect: NSRect) {
        guard let colors else { return }
        let rect = bounds.insetBy(dx: metrics.border, dy: metrics.border)
        uiColor(isHighlighted ? colors.surface : colors.raised).setFill()
        let path = NSBezierPath(roundedRect: rect, xRadius: metrics.radius, yRadius: metrics.radius)
        path.fill()
        uiColor(selected || window?.firstResponder === self ? colors.focus : colors.border).setStroke()
        path.lineWidth = selected ? 2 * metrics.border : metrics.border
        path.stroke()
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = userInterfaceLayoutDirection == .rightToLeft ? .right : .left
        let padding = metrics.padding
        let width = bounds.width - 2 * padding
        (caption as NSString).draw(in: NSRect(x: padding, y: bounds.height - padding - type.label - metrics.small,
            width: width, height: type.label + metrics.small), withAttributes: [.font: NSFont.systemFont(ofSize: type.label),
            .foregroundColor: uiColor(colors.secondary), .paragraphStyle: paragraph])
        (MetricFormat.isolate(reading) as NSString).draw(in: NSRect(x: padding, y: padding,
            width: width, height: type.card + metrics.small), withAttributes: [.font: NSFont.monospacedDigitSystemFont(ofSize: type.card, weight: .medium),
            .foregroundColor: uiColor(colors.text), .paragraphStyle: paragraph])
    }
}

@MainActor
private final class HistoryChart: NSView {
    var view: AnalysisView?
    var colors: UIResource.Colors!
    var width: Double = 1
    override var isFlipped: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        guard let colors else { return }
        uiColor(colors.raised).setFill(); bounds.fill()
        let area = bounds.insetBy(dx: 2 * width, dy: 2 * width)
        let grid = NSBezierPath()
        grid.lineWidth = width
        for n in 0...4 {
            let y = area.minY + area.height * Double(n) / 4
            grid.move(to: NSPoint(x: area.minX, y: y)); grid.line(to: NSPoint(x: area.maxX, y: y))
        }
        uiColor(colors.border).setStroke(); grid.stroke()
        guard let view, view.yMax > view.yMin else { return }
        let line = NSBezierPath(); line.lineWidth = 2 * width
        let peaks = NSBezierPath(); peaks.lineWidth = width
        var last: NSPoint?
        func point(_ x: Double, _ y: Double) -> NSPoint {
            NSPoint(x: area.minX + area.width * max(0, min(1, 1 - (view.endMs - x) / (view.rangeSeconds * 1000))),
                y: area.maxY - area.height * max(0, min(1, (y - view.yMin) / (view.yMax - view.yMin))))
        }
        for p in view.points {
            let current = point(p.xMs, p.value)
            if let last, !p.gap { line.move(to: last); line.line(to: current) }
            peaks.move(to: point(p.xMs, p.low)); peaks.line(to: point(p.xMs, p.high))
            // A single sample remains visible without inventing an interval.
            peaks.move(to: NSPoint(x: current.x - width, y: current.y)); peaks.line(to: NSPoint(x: current.x + width, y: current.y))
            last = current
        }
        uiColor(colors.accent).setStroke(); peaks.stroke(); line.stroke()
    }
}

@MainActor
private final class AnalysisSurface: NSView {
    var appearanceChanged: (() -> Void)?
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        appearanceChanged?()
    }
}

@MainActor
final class AnalysisWindow: NSWindowController, NSWindowDelegate {
    private let resource: UIResource
    private var config: AppConfig
    private var language: String
    private let cli: ConfigCLI
    private var reader: AnalysisReader?
    private var generation: UInt64 = 0
    private var view: AnalysisView?
    private var raw: Data?
    private var selected: String?
    private var range: Int
    private var paused = false
    private var corePage = 0
    private var live = false
    private var cells: [(UIResource.Cell, MetricCell)] = []
    private var coreCells: [(UInt32, MetricCell)] = []
    private var coreIDs: [UInt32] = []
    private var gpuIDs: [String] = []
    private var root = NSStackView()
    private var detail = NSStackView()
    private var coreBox = NSStackView()
    private var gpuBox = NSStackView()
    private var chart = HistoryChart()
    private var status = NSTextField(labelWithString: "")
    private var coverage = NSTextField(labelWithString: "")
    private var stats = NSTextField(labelWithString: "")
    private var pageLabel = NSTextField(labelWithString: "")
    private var scale = NSTextField(labelWithString: "")
    private var pauseButton = NSButton()
    private var series = NSPopUpButton()
    private var onClose: (() -> Void)?
    private var contrastObserver: NSObjectProtocol?
    private var retry: Timer?
    init(resource: UIResource, config: AppConfig, language: String, cli: ConfigCLI, onClose: @escaping () -> Void) {
        self.resource = resource; self.config = config; self.language = language; self.cli = cli
        range = resource.analysis.historySeconds
        self.onClose = onClose
        let m = resource.metrics[config.display.density]!
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: m.minWidth * 1.5, height: m.minWidth * 1.5),
            styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.title = resource.text("app_title", language: language)
        window.minSize = NSSize(width: m.minWidth + 2 * m.padding, height: m.minWidth)
        super.init(window: window)
        window.delegate = self
        build()
        window.center()
        contrastObserver = NSWorkspace.shared.notificationCenter.addObserver(forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
            object: nil, queue: .main) { [weak self] _ in Task { @MainActor in self?.applyAppearance() } }
    }
    required init?(coder: NSCoder) { nil }
    func open(metric: String? = nil) {
        if let metric { selected = metric }
        build()
        showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
        restart()
    }
    func configure(_ config: AppConfig, language: String) {
        self.config = config; self.language = language
        if window?.isVisible == true { build(); restart() }
    }
    func windowWillClose(_ notification: Notification) {
        generation &+= 1; reader?.stop(); reader = nil
        retry?.invalidate(); retry = nil
        view = nil; raw = nil
        if let contrastObserver { NSWorkspace.shared.notificationCenter.removeObserver(contrastObserver) }
        contrastObserver = nil
        onClose?(); onClose = nil
    }
    private func text(_ key: String) -> String { resource.text(key, language: language) }
    private func button(_ key: String, action: Selector) -> NSButton {
        let b = NSButton(title: text(key), target: self, action: action)
        b.bezelStyle = .rounded
        b.font = NSFont.systemFont(ofSize: resource.typography[language]!.label)
        return b
    }
    private func stack(_ orientation: NSUserInterfaceLayoutOrientation = .vertical) -> NSStackView {
        let s = NSStackView(); s.orientation = orientation
        s.spacing = resource.metrics[config.display.density]!.gap
        s.alignment = orientation == .vertical ? .leading : .centerY
        s.distribution = orientation == .horizontal ? .fillEqually : .fill
        return s
    }
    private func full(_ child: NSView, in parent: NSStackView) {
        parent.addArrangedSubview(child)
        child.widthAnchor.constraint(equalTo: parent.widthAnchor).isActive = true
    }
    private func build() {
        guard let window else { return }
        let m = resource.metrics[config.display.density]!
        let host = AnalysisSurface(); host.wantsLayer = true
        host.appearanceChanged = { [weak self] in self?.applyAppearance() }
        host.userInterfaceLayoutDirection = language == "ar" ? .rightToLeft : .leftToRight
        root = stack(); root.translatesAutoresizingMaskIntoConstraints = false
        host.addSubview(root)
        NSLayoutConstraint.activate([root.leadingAnchor.constraint(equalTo: host.leadingAnchor, constant: m.padding),
            root.trailingAnchor.constraint(equalTo: host.trailingAnchor, constant: -m.padding),
            root.topAnchor.constraint(equalTo: host.topAnchor, constant: m.padding),
            root.bottomAnchor.constraint(equalTo: host.bottomAnchor, constant: -m.padding)])
        window.contentView = host
        let header = stack(.horizontal)
        let title = NSTextField(labelWithString: text("app_title"))
        title.font = NSFont.systemFont(ofSize: resource.typography[language]!.body, weight: .semibold)
        header.addArrangedSubview(title)
        header.addArrangedSubview(button("settings", action: #selector(settings)))
        full(header, in: root)
        status = NSTextField(labelWithString: text("loading")); full(status, in: root)
        cells = []
        let overview = stack()
        for offset in stride(from: 0, to: resource.cells.count, by: 2) {
            let row = stack(.horizontal)
            for cell in resource.cells[offset..<min(offset + 2, resource.cells.count)] {
                let b = MetricCell(caption: text(cell.label), reading: MetricFormat.absent, resource: resource,
                    config: config, language: language) { [weak self] in self?.select(cell.metric) }
                row.addArrangedSubview(b); cells.append((cell, b))
            }
            full(row, in: overview)
        }
        full(overview, in: root)
        overview.isHidden = selected != nil
        detail = stack()
        if selected != nil {
            let actions = stack(.horizontal)
            actions.addArrangedSubview(button("back", action: #selector(back)))
            pauseButton = button(paused ? "resume" : "pause", action: #selector(togglePause))
            actions.addArrangedSubview(pauseButton)
            actions.addArrangedSubview(button("copy_chart", action: #selector(copyChart)))
            full(actions, in: detail)
            let controls = stack(.horizontal)
            series = NSPopUpButton()
            series.addItems(withTitles: resource.series.map { text($0.label) })
            if let index = resource.series.firstIndex(where: { $0.key == selected }) { series.selectItem(at: index) }
            series.target = self; series.action = #selector(changeSeries)
            series.setAccessibilityLabel(text("readings"))
            controls.addArrangedSubview(series)
            let ranges = NSPopUpButton()
            ranges.addItems(withTitles: resource.analysis.rangesSeconds.map { text("range_\($0 / 60)") })
            ranges.selectItem(at: resource.analysis.rangesSeconds.firstIndex(of: range) ?? 0)
            ranges.target = self; ranges.action = #selector(changeRange(_:))
            ranges.setAccessibilityLabel(text("range")); controls.addArrangedSubview(ranges)
            full(controls, in: detail)
            scale = NSTextField(labelWithString: ""); full(scale, in: detail)
            chart = HistoryChart(); chart.translatesAutoresizingMaskIntoConstraints = false
            chart.setAccessibilityElement(true); chart.setAccessibilityRole(.image)
            full(chart, in: detail)
            chart.heightAnchor.constraint(greaterThanOrEqualToConstant: m.minWidth / 2).isActive = true
            let axis = stack(.horizontal)
            axis.addArrangedSubview(NSTextField(labelWithString: "−\(range / 60) min"))
            let now = NSTextField(labelWithString: "0"); now.alignment = .right; axis.addArrangedSubview(now)
            full(axis, in: detail)
            stats = NSTextField(wrappingLabelWithString: ""); full(stats, in: detail)
            coverage = NSTextField(wrappingLabelWithString: ""); full(coverage, in: detail)
            coreBox = stack(); full(coreBox, in: detail)
            gpuBox = stack(); full(gpuBox, in: detail)
            let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.drawsBackground = false
            detail.translatesAutoresizingMaskIntoConstraints = false
            let clip = scroll.contentView
            clip.documentView = detail
            NSLayoutConstraint.activate([detail.leadingAnchor.constraint(equalTo: clip.leadingAnchor),
                detail.trailingAnchor.constraint(equalTo: clip.trailingAnchor), detail.topAnchor.constraint(equalTo: clip.topAnchor)])
            full(scroll, in: root)
            scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: m.minWidth).isActive = true
            coreIDs = []; gpuIDs = []
        }
        let spacer = NSView(); spacer.setContentHuggingPriority(.defaultLow, for: .vertical)
        full(spacer, in: root)
        let hint = NSTextField(wrappingLabelWithString: text("history_hint"))
        hint.font = NSFont.systemFont(ofSize: resource.typography[language]!.label)
        full(hint, in: root)
        let footer = stack(.horizontal)
        let website = button("website", action: #selector(website))
        website.toolTip = text("website_help").replacingOccurrences(of: "{url}", with: resource.website)
        footer.addArrangedSubview(website); footer.addArrangedSubview(button("close", action: #selector(closeWindow)))
        full(footer, in: root)
        window.appearance = config.display.theme == "system" ? nil : NSAppearance(named: config.display.theme.hasPrefix("dark") ? .darkAqua : .aqua)
        applyAppearance()
        render()
    }
    private func restart() {
        retry?.invalidate(); retry = nil
        generation &+= 1; reader?.stop(); reader = nil
        guard !paused, window?.isVisible == true else { return }
        live = false; status.stringValue = text("loading")
        let current = generation
        reader = AnalysisReader(cli: cli, resource: resource, metric: selected ?? "cpu.usage", range: range) { [weak self] event in
            Task { @MainActor in
                guard let self, self.generation == current else { return }
                switch event {
                case .view(let view, let raw): self.view = view; self.raw = raw; self.live = true
                case .offline:
                    self.live = false
                    self.retry = Timer.scheduledTimer(timeInterval: Double(self.config.transport.reconnectSeconds),
                        target: self, selector: #selector(self.reconnectAnalysis), userInfo: nil, repeats: false)
                }
                self.render()
            }
        }
    }
    @objc private func reconnectAnalysis() { restart() }
    private func format(_ value: Double?, unit: String) -> String {
        guard let value else { return MetricFormat.absent }
        switch unit {
        case "percent": return MetricFormat.percent(value)
        case "celsius": return MetricFormat.temperature(value)
        case "bytes": return ByteCountFormatter.string(fromByteCount: Int64(min(value, Double(Int64.max) - 2048)), countStyle: .binary)
        default: return MetricFormat.rate(UInt64(max(0, min(value, Double(UInt64.max) / 2))))
        }
    }
    private func render() {
        status.stringValue = text(paused ? "paused" : live ? "connected" : "disconnected")
        let readings = Dictionary(uniqueKeysWithValues: MetricFormat.readings(live || paused ? view?.latest?.sample : nil))
        for (cell, button) in cells {
            let label = resource.series.first(where: { $0.key == cell.metric })?.label ?? cell.label
            button.update(cell.label == "history" ? "\(Int((view?.availableMs ?? 0) / 60000)) min" : readings[label] ?? MetricFormat.absent)
        }
        guard selected != nil, let view else { return }
        chart.view = view; chart.needsDisplay = true
        let s = view.statistics
        stats.stringValue = [("current", live || paused ? s.current : nil), ("minimum", s.minimum),
            ("average", s.average), ("maximum", s.maximum)].map { "\(text($0.0)): \(MetricFormat.isolate(format($0.1, unit: view.unit)))" }.joined(separator: "     ")
        chart.setAccessibilityLabel("\(text(view.label).replacingOccurrences(of: "{id}", with: selected?.replacingOccurrences(of: "core:", with: "") ?? "")); \(stats.stringValue)")
        coverage.stringValue = text("history_coverage").replacingOccurrences(of: "{count}", with: String(s.samples))
            .replacingOccurrences(of: "{minutes}", with: String(format: "%.1f", s.coveredMs / 60000))
        scale.stringValue = "\(format(view.yMax, unit: view.unit))  —  \(format(view.yMin, unit: view.unit))"
        renderCores(view.latest?.detail.cores ?? [])
        renderGPUs(view.latest?.detail)
    }
    private func clear(_ stack: NSStackView) { for v in stack.arrangedSubviews { stack.removeArrangedSubview(v); v.removeFromSuperview() } }
    private func renderCores(_ cores: [AnalysisView.Core]) {
        guard selected == "cpu.usage" || selected?.hasPrefix("core:") == true else { coreBox.isHidden = true; return }
        coreBox.isHidden = false
        let size = resource.analysis.coresPerPage
        corePage = min(corePage, max(0, (cores.count - 1) / size))
        let slice = Array(cores.dropFirst(corePage * size).prefix(size))
        if coreIDs != slice.map(\.id) || coreCells.isEmpty {
            clear(coreBox); coreCells = []; coreIDs = slice.map(\.id)
            let navigation = stack(.horizontal)
            navigation.addArrangedSubview(button("previous", action: #selector(previousPage)))
            pageLabel = NSTextField(labelWithString: "")
            navigation.addArrangedSubview(pageLabel)
            navigation.addArrangedSubview(button("next", action: #selector(nextPage)))
            full(navigation, in: coreBox)
            for offset in stride(from: 0, to: slice.count, by: 4) {
                let row = stack(.horizontal)
                for core in slice[offset..<min(offset + 4, slice.count)] {
                    let b = MetricCell(caption: text("logical_core").replacingOccurrences(of: "{id}", with: String(core.id)),
                        reading: "", resource: resource, config: config, language: language) { [weak self] in self?.select("core:\(core.id)") }
                    row.addArrangedSubview(b); coreCells.append((core.id, b))
                }
                full(row, in: coreBox)
            }
            applyAppearance()
        }
        pageLabel.stringValue = text("core_page").replacingOccurrences(of: "{first}", with: String(cores.isEmpty ? 0 : corePage * size + 1))
            .replacingOccurrences(of: "{last}", with: String(min(cores.count, (corePage + 1) * size)))
            .replacingOccurrences(of: "{count}", with: String(cores.count))
        for (id, b) in coreCells { b.selected = selected == "core:\(id)"; b.update(MetricFormat.percent(live || paused ? slice.first(where: { $0.id == id })?.usage : nil)) }
    }
    private func renderGPUs(_ detail: AnalysisView.Detail?) {
        guard selected?.hasPrefix("gpu") == true else { gpuBox.isHidden = true; return }
        gpuBox.isHidden = false
        let gpus = detail?.gpus ?? []
        let ids = gpus.map { "\($0.id):\($0.status)" }
        guard ids != gpuIDs || gpuBox.arrangedSubviews.isEmpty else { return }
        gpuIDs = ids; clear(gpuBox)
        if detail?.gpuEnabled == false {
            full(NSTextField(labelWithString: text("gpu_disabled")), in: gpuBox)
            full(button("enable_gpu", action: #selector(enableGPU)), in: gpuBox)
        }
        for gpu in gpus {
            let title = NSTextField(labelWithString: "\(gpu.name) · \(gpu.status == "available" ? text("connected") : text("unavailable"))")
            full(title, in: gpuBox)
            let row = stack(.horizontal)
            for (field, label) in [("usage", "gpu"), ("temperature", "gpu_temperature"),
                ("dedicated_used", "dedicated_memory"), ("shared_used", "shared_memory")] {
                let b = NSButton(title: text(label), target: nil, action: nil)
                b.target = self; b.action = #selector(selectGPU(_:)); b.identifier = NSUserInterfaceItemIdentifier("gpu:\(gpu.id):\(field)")
                row.addArrangedSubview(b)
            }
            full(row, in: gpuBox)
        }
    }
    private func select(_ key: String) {
        selected = key; paused = false
        let m = resource.metrics[config.display.density]!
        if let window {
            let width = min(NSScreen.main?.visibleFrame.width ?? m.minWidth * 3, m.minWidth * 3)
            window.setContentSize(NSSize(width: width, height: min(NSScreen.main?.visibleFrame.height ?? 720, m.minWidth * 2.25)))
        }
        build(); restart()
    }
    @objc private func back() { selected = nil; build(); restart() }
    @objc private func changeSeries() { select(resource.series[series.indexOfSelectedItem].key) }
    @objc private func changeRange(_ sender: NSPopUpButton) { range = resource.analysis.rangesSeconds[sender.indexOfSelectedItem]; restart() }
    @objc private func selectGPU(_ sender: NSButton) { if let key = sender.identifier?.rawValue { select(key) } }
    @objc private func togglePause() {
        paused.toggle(); pauseButton.title = text(paused ? "resume" : "pause")
        restart(); render()
    }
    @objc private func previousPage() { corePage = max(0, corePage - 1); coreIDs = []; render() }
    @objc private func nextPage() { corePage += 1; coreIDs = []; render() }
    @objc private func copyChart() {
        guard let raw else { return }
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(String(decoding: raw, as: UTF8.self), forType: .string)
    }
    @objc private func website() { if let url = URL(string: resource.website) { NSWorkspace.shared.open(url) } }
    @objc private func closeWindow() { window?.close() }
    @objc private func settings() { NSApp.sendAction(Selector(("openSettingsMenu:")), to: NSApp.delegate, from: self) }
    @objc private func enableGPU() { Task { _ = try? await cli.run(["config", "set", "sampling.gpu", "true"]); _ = try? await cli.run(["service", "apply"]); restart() } }
    @objc private func appearanceChanged() { applyAppearance() }
    private func applyAppearance() {
        guard let window else { return }
        let dark = window.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        var theme = config.display.theme == "system" ? (dark ? "dark" : "light") : config.display.theme
        if NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast && !theme.hasSuffix("-contrast") { theme += "-contrast" }
        let colors = resource.themes[theme]!
        window.backgroundColor = uiColor(colors.surface)
        window.contentView?.layer?.backgroundColor = uiColor(colors.surface).cgColor
        func style(_ view: NSView) {
            if let label = view as? NSTextField { label.textColor = uiColor(colors.text) }
            if let cell = view as? MetricCell { cell.colors = colors; cell.needsDisplay = true }
            for child in view.subviews { style(child) }
        }
        if let content = window.contentView { style(content) }
        chart.colors = colors; chart.width = resource.metrics[config.display.density]!.border; chart.needsDisplay = true
    }
    #if UI_ACCEPTANCE
    var acceptanceView: AnalysisView? { view }
    var acceptancePaused: Bool { paused }
    var acceptanceHasReader: Bool { reader != nil }
    func acceptanceSelect(_ metric: String) { select(metric) }
    func acceptancePause() { togglePause() }
    #endif
}
