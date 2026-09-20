#!/usr/bin/env swift
import AppKit
import CoreText
import Foundation
import ImageIO
import UniformTypeIdentifiers

// Layout must match `src-tauri/tauri.conf.json` → bundle.macOS.dmg.
// Coordinates are Finder points (origin top-left, icon position = icon center).
// Window is 660×428 so the 28px title bar sits above this 660×400 art.
let windowW: CGFloat = 660
let windowH: CGFloat = 400
let appX: CGFloat = 168
let appY: CGFloat = 178
let appsX: CGFloat = 492
let appsY: CGFloat = 178
let iconPt: CGFloat = 128
let scale: CGFloat = 2

let cream = rgb(0xF8, 0xE4, 0xCF)
let creamDeep = rgb(0xF0, 0xD8, 0xBE)
let creamHot = rgb(0xFD, 0xF0, 0xE2)
let navy = rgb(0x1B, 0x3A, 0x6C)
let navyInk = rgb(0x12, 0x2A, 0x52)
let sky = rgb(0x6F, 0xA1, 0xE4)
let caption = rgb(0x6D, 0x5E, 0x50)

let here = URL(fileURLWithPath: CommandLine.arguments[0])
  .standardizedFileURL.deletingLastPathComponent()
let outBackground = here.appendingPathComponent("background.png")
let outPreview = here.appendingPathComponent("preview.png")
let wantPreview = CommandLine.arguments.contains("--preview")

let width = Int(windowW * scale)
let height = Int(windowH * scale)
let colorSpace = CGColorSpaceCreateDeviceRGB()
guard let ctx = CGContext(
  data: nil,
  width: width,
  height: height,
  bitsPerComponent: 8,
  bytesPerRow: 0,
  space: colorSpace,
  bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
) else {
  fputs("Could not create bitmap context\n", stderr)
  exit(1)
}

ctx.setShouldAntialias(true)
ctx.setAllowsAntialiasing(true)
ctx.interpolationQuality = .high

paintBackground(ctx)
paintGrain(ctx)
paintArrow(ctx)
paintCaption(ctx)

guard let backgroundImage = ctx.makeImage() else {
  fputs("Could not snapshot background\n", stderr)
  exit(1)
}

writePNG(backgroundImage, to: outBackground)
print("Wrote \(outBackground.path)")

if wantPreview {
  guard let preview = renderPreview(background: backgroundImage) else {
    fputs("Could not render preview\n", stderr)
    exit(1)
  }
  writePNG(preview, to: outPreview)
  print("Wrote \(outPreview.path)")
}

// MARK: - Drawing

func paintBackground(_ ctx: CGContext) {
  let gradient = CGGradient(
    colorsSpace: colorSpace,
    colors: [creamHot, cream, creamDeep] as CFArray,
    locations: [0, 0.42, 1]
  )!
  ctx.drawLinearGradient(
    gradient,
    start: CGPoint(x: CGFloat(width) * 0.5, y: CGFloat(height)),
    end: CGPoint(x: CGFloat(width) * 0.5, y: 0),
    options: [.drawsBeforeStartLocation, .drawsAfterEndLocation]
  )

  ctx.saveGState()
  ctx.setBlendMode(.multiply)
  let vignette = CGGradient(
    colorsSpace: colorSpace,
    colors: [
      rgb(0xF8, 0xE4, 0xCF).copy(alpha: 0)!,
      rgb(0xC9, 0xA8, 0x86).copy(alpha: 0.10)!,
    ] as CFArray,
    locations: [0.55, 1]
  )!
  ctx.drawRadialGradient(
    vignette,
    startCenter: CGPoint(x: CGFloat(width) * 0.5, y: fy(appY) + 8),
    startRadius: 0,
    endCenter: CGPoint(x: CGFloat(width) * 0.5, y: fy(appY)),
    endRadius: CGFloat(width) * 0.72,
    options: [.drawsAfterEndLocation]
  )
  ctx.restoreGState()

  ctx.saveGState()
  ctx.setBlendMode(.plusLighter)
  let glow = CGGradient(
    colorsSpace: colorSpace,
    colors: [
      sky.copy(alpha: 0.14)!,
      sky.copy(alpha: 0)!,
    ] as CFArray,
    locations: [0, 1]
  )!
  ctx.drawRadialGradient(
    glow,
    startCenter: p((appX + appsX) / 2, appY - 18),
    startRadius: 0,
    endCenter: p((appX + appsX) / 2, appY - 18),
    endRadius: 220,
    options: []
  )
  ctx.restoreGState()
}

func paintGrain(_ ctx: CGContext) {
  var rng = SplitMix64(seed: 0x5E80_57D6_2026_0D06)
  ctx.setFillColor(rgb(0x3A, 0x2A, 0x18).copy(alpha: 0.045)!)
  for _ in 0..<18_000 {
    let x = CGFloat(rng.next() % UInt64(width))
    let y = CGFloat(rng.next() % UInt64(height))
    let size: CGFloat = rng.next() % 3 == 0 ? 1.6 : 1.0
    ctx.fill(CGRect(x: x, y: y, width: size, height: size))
  }
  ctx.setFillColor(NSColor.white.withAlphaComponent(0.05).cgColor)
  for _ in 0..<5_000 {
    let x = CGFloat(rng.next() % UInt64(width))
    let y = CGFloat(rng.next() % UInt64(height))
    ctx.fill(CGRect(x: x, y: y, width: 1, height: 1))
  }
}

func paintArrow(_ ctx: CGContext) {
  let inset: CGFloat = 16
  let start = CGPoint(x: appX + iconPt / 2 + inset, y: appY - 4)
  let end = CGPoint(x: appsX - iconPt / 2 - inset, y: appsY - 4)
  let rise: CGFloat = 36
  let curve = Cubic(
    p0: start,
    p1: CGPoint(x: lerp(start.x, end.x, 0.30), y: start.y - rise),
    p2: CGPoint(x: lerp(start.x, end.x, 0.70), y: end.y - rise),
    p3: end
  )

  let tangent = normalize(curve.derivative(1))
  let normal = CGPoint(x: -tangent.y, y: tangent.x)
  let headLen: CGFloat = 32
  let headHalf: CGFloat = 18
  let tip = end
  let headBase = CGPoint(
    x: tip.x - tangent.x * headLen,
    y: tip.y - tangent.y * headLen
  )
  let left = CGPoint(
    x: headBase.x + normal.x * headHalf - tangent.x * 3,
    y: headBase.y + normal.y * headHalf - tangent.y * 3
  )
  let right = CGPoint(
    x: headBase.x - normal.x * headHalf - tangent.x * 3,
    y: headBase.y - normal.y * headHalf - tangent.y * 3
  )

  let shaftEndT: CGFloat = 0.82
  let shaft = CGMutablePath()
  shaft.move(to: p(curve.point(0)))
  stride(from: CGFloat(0.02), through: shaftEndT, by: 0.02).forEach { t in
    shaft.addLine(to: p(curve.point(t)))
  }

  let head = CGMutablePath()
  head.move(to: p(tip))
  head.addLine(to: p(left))
  head.addLine(to: p(right))
  head.closeSubpath()

  let stroke = 9 * scale
  ctx.saveGState()
  ctx.setShadow(
    offset: CGSize(width: 0, height: -4),
    blur: 10,
    color: rgb(0x2A, 0x1C, 0x10).copy(alpha: 0.24)
  )
  ctx.setStrokeColor(navyInk)
  ctx.setFillColor(navyInk)
  ctx.setLineCap(.round)
  ctx.setLineJoin(.round)
  ctx.setLineWidth(stroke + 1.5)
  ctx.addPath(shaft)
  ctx.strokePath()
  ctx.addPath(head)
  ctx.fillPath()
  ctx.restoreGState()

  ctx.setStrokeColor(navy)
  ctx.setFillColor(navy)
  ctx.setLineCap(.round)
  ctx.setLineJoin(.round)
  ctx.setLineWidth(stroke)
  ctx.addPath(shaft)
  ctx.strokePath()
  ctx.addPath(head)
  ctx.fillPath()

  ctx.saveGState()
  ctx.setBlendMode(.screen)
  ctx.setStrokeColor(sky.copy(alpha: 0.48)!)
  ctx.setLineCap(.round)
  ctx.setLineWidth(4)
  let highlight = CGMutablePath()
  highlight.move(to: p(offset(curve.point(0.03), by: CGPoint(x: 0, y: -4))))
  stride(from: CGFloat(0.05), through: 0.74, by: 0.02).forEach { t in
    highlight.addLine(to: p(offset(curve.point(t), by: CGPoint(x: 0, y: -4))))
  }
  ctx.addPath(highlight)
  ctx.strokePath()
  ctx.restoreGState()
}

func paintCaption(_ ctx: CGContext) {
  let text = "Drag to Applications to install"
  let font = NSFont.systemFont(ofSize: 13 * scale, weight: .medium)
  let paragraph = NSMutableParagraphStyle()
  paragraph.alignment = .center
  let attrs: [NSAttributedString.Key: Any] = [
    .font: font,
    .foregroundColor: NSColor(cgColor: caption.copy(alpha: 0.82)!)!,
    .kern: 0.35 * scale,
    .paragraphStyle: paragraph,
  ]
  let attributed = NSAttributedString(string: text, attributes: attrs)
  let line = CTLineCreateWithAttributedString(attributed)
  let bounds = CTLineGetImageBounds(line, ctx)
  let x = (CGFloat(width) - bounds.width) / 2
  let y = fy(348) - bounds.height / 2
  ctx.textPosition = CGPoint(x: x, y: y)
  CTLineDraw(line, ctx)
}

func renderPreview(background: CGImage) -> CGImage? {
  guard let ctx = CGContext(
    data: nil,
    width: width,
    height: height,
    bitsPerComponent: 8,
    bytesPerRow: 0,
    space: colorSpace,
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  ) else { return nil }

  ctx.draw(background, in: CGRect(x: 0, y: 0, width: width, height: height))
  NSGraphicsContext.saveGraphicsState()
  NSGraphicsContext.current = NSGraphicsContext(cgContext: ctx, flipped: false)

  drawFinderIcon(
    image: appIconImage(),
    center: CGPoint(x: appX, y: appY),
    label: "Rebost"
  )
  drawFinderIcon(
    image: NSWorkspace.shared.icon(forFile: "/Applications"),
    center: CGPoint(x: appsX, y: appsY),
    label: "Applications"
  )

  NSGraphicsContext.restoreGraphicsState()
  return ctx.makeImage()
}

func drawFinderIcon(image: NSImage, center: CGPoint, label: String) {
  let size = iconPt * scale
  let rect = CGRect(
    x: fx(center.x) - size / 2,
    y: fy(center.y) - size / 2,
    width: size,
    height: size
  )
  image.size = NSSize(width: iconPt, height: iconPt)
  image.draw(
    in: rect,
    from: .zero,
    operation: .sourceOver,
    fraction: 1
  )

  let font = NSFont.systemFont(ofSize: 12 * scale, weight: .regular)
  let paragraph = NSMutableParagraphStyle()
  paragraph.alignment = .center
  let attrs: [NSAttributedString.Key: Any] = [
    .font: font,
    .foregroundColor: NSColor(srgbRed: 0.13, green: 0.16, blue: 0.23, alpha: 0.92),
    .paragraphStyle: paragraph,
  ]
  let attributed = NSAttributedString(string: label, attributes: attrs)
  let line = CTLineCreateWithAttributedString(attributed)
  guard let ctx = NSGraphicsContext.current?.cgContext else { return }
  let bounds = CTLineGetImageBounds(line, ctx)
  ctx.textPosition = CGPoint(
    x: fx(center.x) - bounds.width / 2,
    y: fy(center.y) - size / 2 - 18 * scale
  )
  CTLineDraw(line, ctx)
}

func appIconImage() -> NSImage {
  let candidates = [
    here.deletingLastPathComponent().appendingPathComponent("icons/128x128@2x.png"),
    here.deletingLastPathComponent().appendingPathComponent("icons/icon.png"),
  ]
  for url in candidates where FileManager.default.fileExists(atPath: url.path) {
    if let image = NSImage(contentsOf: url) { return image }
  }
  return NSWorkspace.shared.icon(forFile: "/")
}

// MARK: - Geometry (Finder points → pixel canvas)

func fx(_ x: CGFloat) -> CGFloat { x * scale }

func fy(_ y: CGFloat) -> CGFloat { CGFloat(height) - y * scale }

func p(_ point: CGPoint) -> CGPoint {
  CGPoint(x: fx(point.x), y: fy(point.y))
}

func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { p(CGPoint(x: x, y: y)) }

func lerp(_ a: CGFloat, _ b: CGFloat, _ t: CGFloat) -> CGFloat { a + (b - a) * t }

func offset(_ point: CGPoint, by delta: CGPoint) -> CGPoint {
  CGPoint(x: point.x + delta.x, y: point.y + delta.y)
}

func normalize(_ v: CGPoint) -> CGPoint {
  let len = max(hypot(v.x, v.y), 0.0001)
  return CGPoint(x: v.x / len, y: v.y / len)
}

struct Cubic {
  let p0, p1, p2, p3: CGPoint

  func point(_ t: CGFloat) -> CGPoint {
    let u = 1 - t
    return CGPoint(
      x: u * u * u * p0.x + 3 * u * u * t * p1.x + 3 * u * t * t * p2.x + t * t * t * p3.x,
      y: u * u * u * p0.y + 3 * u * u * t * p1.y + 3 * u * t * t * p2.y + t * t * t * p3.y
    )
  }

  func derivative(_ t: CGFloat) -> CGPoint {
    let u = 1 - t
    return CGPoint(
      x: 3 * u * u * (p1.x - p0.x) + 6 * u * t * (p2.x - p1.x) + 3 * t * t * (p3.x - p2.x),
      y: 3 * u * u * (p1.y - p0.y) + 6 * u * t * (p2.y - p1.y) + 3 * t * t * (p3.y - p2.y)
    )
  }
}

func rgb(_ r: Int, _ g: Int, _ b: Int) -> CGColor {
  CGColor(
    srgbRed: CGFloat(r) / 255,
    green: CGFloat(g) / 255,
    blue: CGFloat(b) / 255,
    alpha: 1
  )
}

struct SplitMix64: RandomNumberGenerator {
  var state: UInt64
  init(seed: UInt64) { state = seed }
  mutating func next() -> UInt64 {
    state &+= 0x9E37_79B9_7F4A_7C15
    var z = state
    z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
    z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
    return z ^ (z >> 31)
  }
}

func writePNG(_ image: CGImage, to url: URL) {
  let data = NSMutableData()
  guard let dest = CGImageDestinationCreateWithData(
    data,
    UTType.png.identifier as CFString,
    1,
    nil
  ) else {
    fputs("Could not create PNG destination\n", stderr)
    exit(1)
  }
  let props: [CFString: Any] = [
    kCGImagePropertyDPIWidth: 144,
    kCGImagePropertyDPIHeight: 144,
    kCGImagePropertyPixelWidth: width,
    kCGImagePropertyPixelHeight: height,
  ]
  CGImageDestinationAddImage(dest, image, props as CFDictionary)
  guard CGImageDestinationFinalize(dest) else {
    fputs("Could not write PNG\n", stderr)
    exit(1)
  }
  do {
    try data.write(to: url)
  } catch {
    fputs("Could not save \(url.path): \(error)\n", stderr)
    exit(1)
  }
}
