import Foundation
import CoreText
import CoreGraphics
let font = CTFontCreateWithName("AvenirNext-DemiBold" as CFString, 160, nil)
let attributes = [kCTFontAttributeName as NSAttributedString.Key: font]
let line = CTLineCreateWithAttributedString(NSAttributedString(string: "SemanticDB", attributes: attributes))
let shape = CGMutablePath()
for run in CTLineGetGlyphRuns(line) as! [CTRun] {
    let n = CTRunGetGlyphCount(run)
    var glyphs = [CGGlyph](repeating: 0, count: n)
    var positions = [CGPoint](repeating: .zero, count: n)
    CTRunGetGlyphs(run, CFRange(location: 0, length: 0), &glyphs)
    CTRunGetPositions(run, CFRange(location: 0, length: 0), &positions)
    for i in 0..<n {
        if let path = CTFontCreatePathForGlyph(font, glyphs[i], nil) {
            shape.addPath(path, transform: CGAffineTransform(translationX: positions[i].x, y: positions[i].y))
        }
    }
}
let bounds = shape.boundingBoxOfPath
var transform = CGAffineTransform(a: 1, b: 0, c: 0, d: -1, tx: -bounds.minX, ty: bounds.maxY)
let normalized = shape.copy(using: &transform)!
func number(_ v: CGFloat) -> String { String(format: "%.3f", Double(v)) }
func point(_ p: CGPoint) -> String { "\(number(p.x)) \(number(p.y))" }
var d = ""
normalized.applyWithBlock { pointer in
    let e = pointer.pointee
    switch e.type {
    case .moveToPoint: d += "M\(point(e.points[0]))"
    case .addLineToPoint: d += "L\(point(e.points[0]))"
    case .addQuadCurveToPoint: d += "Q\(point(e.points[0])) \(point(e.points[1]))"
    case .addCurveToPoint: d += "C\(point(e.points[0])) \(point(e.points[1])) \(point(e.points[2]))"
    case .closeSubpath: d += "Z"
    @unknown default: break
    }
}
let data: [String: Any] = ["font": CTFontCopyPostScriptName(font) as String, "width": bounds.width, "height": bounds.height, "path": d]
let json = try JSONSerialization.data(withJSONObject: data, options: [.prettyPrinted, .sortedKeys])
try json.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
