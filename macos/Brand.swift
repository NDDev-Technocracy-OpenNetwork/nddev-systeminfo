import AppKit

@MainActor
private func brandPath(_ brand: UIResource.Brand, rect: NSRect, flipped: Bool) -> NSBezierPath {
    let result = NSBezierPath()
    for source in brand.paths {
        let path = NSBezierPath()
        let scanner = Scanner(string: source)
        scanner.charactersToBeSkipped = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: ","))
        var move = false
        while !scanner.isAtEnd {
            if scanner.scanString("M") != nil { move = true }
            else if scanner.scanString("L") != nil { move = false }
            else if scanner.scanString("Z") != nil || scanner.scanString("z") != nil { path.close(); continue }
            guard let x = scanner.scanDouble(), let y = scanner.scanDouble() else { break }
            let point = NSPoint(x: x, y: y)
            if move { path.move(to: point); move = false } else { path.line(to: point) }
        }
        result.append(path)
    }
    let transform = AffineTransform(translationByX: rect.minX, byY: flipped ? rect.minY : rect.maxY)
    var scaled = transform
    scaled.scale(x: rect.width / brand.viewBox[2], y: (flipped ? 1 : -1) * rect.height / brand.viewBox[3])
    scaled.translate(x: -brand.viewBox[0], y: -brand.viewBox[1])
    result.transform(using: scaled)
    return result
}

@MainActor
func brandImage(_ brand: UIResource.Brand, size: Double) -> NSImage {
    let image = NSImage(size: NSSize(width: size, height: size))
    image.lockFocus()
    NSColor.black.setFill()
    brandPath(brand, rect: NSRect(x: 0, y: 0, width: size, height: size), flipped: false).fill()
    image.unlockFocus()
    image.isTemplate = true
    return image
}

@MainActor
final class BrandHeader: NSView {
    private let resource: UIResource
    private let config: AppConfig
    private let language: String
    override var isFlipped: Bool { true }
    init(resource: UIResource, config: AppConfig, language: String) {
        self.resource = resource
        self.config = config
        self.language = language
        let metrics = resource.metrics[config.display.density]!
        super.init(frame: NSRect(x: 0, y: 0, width: metrics.minWidth, height: metrics.brand + 2 * metrics.padding))
        setAccessibilityElement(true)
        setAccessibilityRole(.staticText)
        setAccessibilityLabel("\(resource.text("app_title", language: language)) \(resource.version)")
    }
    required init?(coder: NSCoder) { nil }
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }
    override func draw(_ dirtyRect: NSRect) {
        let metrics = resource.metrics[config.display.density]!
        let typography = resource.typography[language]!
        let dark = effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
        var theme = config.display.theme == "system" ? (dark ? "dark" : "light") : config.display.theme
        if NSWorkspace.shared.accessibilityDisplayShouldIncreaseContrast && !theme.hasSuffix("-contrast") { theme += "-contrast" }
        let colors = resource.themes[theme]!
        color(colors.surface).setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: metrics.small, dy: metrics.small),
                     xRadius: metrics.radius, yRadius: metrics.radius).fill()
        let rtl = language == "ar"
        let mark = NSRect(x: rtl ? bounds.width - metrics.padding - metrics.brand : metrics.padding,
                          y: metrics.padding, width: metrics.brand, height: metrics.brand)
        color(colors.accent).setFill()
        brandPath(resource.brand, rect: mark, flipped: true).fill()
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = rtl ? .right : .left
        paragraph.baseWritingDirection = rtl ? .rightToLeft : .leftToRight
        let x = rtl ? metrics.padding : mark.maxX + metrics.gap
        let width = bounds.width - metrics.brand - 2 * metrics.padding - metrics.gap
        let title = resource.text("app_title", language: language) as NSString
        title.draw(in: NSRect(x: x, y: metrics.padding, width: width, height: typography.body + metrics.gap),
            withAttributes: [.font: NSFont.systemFont(ofSize: typography.body, weight: .semibold),
                             .foregroundColor: color(colors.text), .paragraphStyle: paragraph])
        (resource.version as NSString).draw(in: NSRect(x: x, y: metrics.padding + typography.body + metrics.small,
            width: width, height: typography.label + metrics.gap),
            withAttributes: [.font: NSFont.systemFont(ofSize: typography.label),
                             .foregroundColor: color(colors.secondary), .paragraphStyle: paragraph])
    }
    private func color(_ hex: String) -> NSColor {
        let value = UInt32(hex.dropFirst(), radix: 16)!
        return NSColor(srgbRed: Double((value >> 16) & 255) / 255,
                       green: Double((value >> 8) & 255) / 255, blue: Double(value & 255) / 255, alpha: 1)
    }
}
