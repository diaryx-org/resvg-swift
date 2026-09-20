// The replayer: a display list into a CGContext, one op at a time.
//
// This is `usvg_flatten::replay` written against CoreGraphics — the same match,
// arm for arm, which is what makes it easy to hold the two to the same picture.
// It knows nothing about SVG; the list has resolved everything, and every
// coordinate here is either the list's canvas or an op's own local space.
//
// The list is y-down, as SVG is. The `transform` a caller passes maps canvas
// units onto the context's *current* user space, and it is the caller's job to
// have made that y-down too (a UIView, a flipped NSView, a PDF page after a
// flip). Images are the one place CoreGraphics insists on y-up, so they are
// flipped back locally where they are drawn.

import CoreGraphics
import Foundation
import ImageIO
import ResvgFFI

extension CGContext {
    /// Draw `list` under `transform` (canvas units → the context's user space).
    ///
    /// Balanced `pushLayer`/`popLayer` is assumed, which `SvgDocument` always
    /// produces; the graphics state is saved and restored around each layer.
    public func draw(_ list: DisplayList, transform root: CGAffineTransform) {
        for op in list.ops {
            switch op {
            case .pushLayer(let layer):
                saveGState()
                for clip in layer.clips { apply(clip, root: root) }
                // Alpha and blend mode set *before* the layer begins are how it
                // composites; inside, CoreGraphics resets both to defaults.
                setAlpha(CGFloat(layer.opacity))
                setBlendMode(layer.blend.cg)
                beginTransparencyLayer(auxiliaryInfo: nil)
            case .popLayer:
                endTransparencyLayer()
                restoreGState()
            case .fill(let op):
                fill(op, root: root)
            case .stroke(let op):
                stroke(op, root: root)
            case .image(let op):
                draw(op, root: root)
            case .raster(let op):
                draw(op, root: root)
            }
        }
    }

    // MARK: Fill and stroke

    private func fill(_ op: FillOp, root: CGAffineTransform) {
        saveGState()
        defer { restoreGState() }
        concatenate(op.transform.cg.concatenating(root))
        setShouldAntialias(op.antialias)
        addPath(op.path.cg)
        switch op.paint {
        case .solid(let color):
            setFillColor(color.cg(alpha: op.opacity))
            fillPath(using: op.rule.cg)
        case .linear, .radial:
            // A gradient fills the path by clipping to it and painting the
            // gradient across the clip. Stop alpha already carries the op's
            // opacity, so it is not applied again here.
            clip(using: op.rule.cg)
            paintGradient(op.paint)
        }
    }

    private func stroke(_ op: StrokeOp, root: CGAffineTransform) {
        saveGState()
        defer { restoreGState() }
        // Transform first, pen second: the width is in local units and
        // stretches with the path, as SVG strokes do.
        concatenate(op.transform.cg.concatenating(root))
        setShouldAntialias(op.antialias)
        setLineWidth(CGFloat(op.stroke.width))
        setLineCap(op.stroke.cap.cg)
        setLineJoin(op.stroke.join.cg)
        setMiterLimit(CGFloat(op.stroke.miterLimit))
        if let dash = op.stroke.dash {
            setLineDash(phase: CGFloat(dash.offset), lengths: dash.array.map { CGFloat($0) })
        }
        addPath(op.path.cg)
        switch op.paint {
        case .solid(let color):
            setStrokeColor(color.cg(alpha: op.opacity))
            strokePath()
        case .linear, .radial:
            // CoreGraphics cannot stroke with a gradient directly; the stroke's
            // outline becomes the clip and the gradient is painted through it.
            replacePathWithStrokedPath()
            clip()
            paintGradient(op.paint)
        }
    }

    /// Paint a gradient across the current clip, in the current user space
    /// (the path's local space) under the gradient's own transform.
    private func paintGradient(_ paint: Paint) {
        let options: CGGradientDrawingOptions = [.drawsBeforeStartLocation, .drawsAfterEndLocation]
        switch paint {
        case .linear(let g):
            guard let gradient = CGGradient.make(g.stops) else { return }
            concatenate(g.transform.cg)
            drawLinearGradient(
                gradient,
                start: CGPoint(x: CGFloat(g.x1), y: CGFloat(g.y1)),
                end: CGPoint(x: CGFloat(g.x2), y: CGFloat(g.y2)),
                options: options)
        case .radial(let g):
            guard let gradient = CGGradient.make(g.stops) else { return }
            concatenate(g.transform.cg)
            drawRadialGradient(
                gradient,
                startCenter: CGPoint(x: CGFloat(g.fx), y: CGFloat(g.fy)),
                startRadius: 0,
                endCenter: CGPoint(x: CGFloat(g.cx), y: CGFloat(g.cy)),
                endRadius: CGFloat(g.r),
                options: options)
        case .solid:
            break
        }
    }

    // MARK: Clipping

    /// Intersect the current clip with `clip`: the union of its shapes.
    ///
    /// One shape is the common case and is exact — a path clip, still vector in
    /// a PDF. Several shapes are a union, which CoreGraphics has no path
    /// operation for: concatenating them into one path is only a union when
    /// they do not overlap (overlaps would cancel or double under either fill
    /// rule), so that is done when their bounding boxes are disjoint and they
    /// share a rule, and otherwise the union is rasterized into a mask at the
    /// device resolution of this context.
    private func apply(_ region: Clip, root: CGAffineTransform) {
        let shapes = region.shapes
        guard let first = shapes.first else {
            // An empty clipPath clips everything away.
            clip(to: CGRect.null)
            return
        }
        if shapes.count == 1 {
            addPath(first.path.cg.placed(under: first.transform.cg.concatenating(root)))
            clip(using: first.rule.cg)
            return
        }
        let paths = shapes.map { ($0.path.cg, $0.transform.cg.concatenating(root)) }
        let sameRule = shapes.allSatisfy { $0.rule == first.rule }
        if sameRule, paths.disjoint {
            let union = CGMutablePath()
            for (path, transform) in paths { union.addPath(path, transform: transform) }
            addPath(union)
            clip(using: first.rule.cg)
            return
        }
        clipToMask(of: shapes, root: root)
    }

    /// Rasterize the union of `shapes` into an alpha mask at this context's
    /// device resolution and clip to it.
    private func clipToMask(of shapes: [ClipShape], root: CGAffineTransform) {
        let device = ctm
        // The union's extent in device pixels is the mask's size.
        var bounds = CGRect.null
        for shape in shapes {
            let toDevice = shape.transform.cg.concatenating(root).concatenating(device)
            bounds = bounds.union(shape.path.cg.boundingBoxOfPath.applying(toDevice))
        }
        let pixels = bounds.integral
        guard pixels.width >= 1, pixels.height >= 1,
            let mask = CGContext(
                data: nil, width: Int(pixels.width), height: Int(pixels.height),
                bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceGray(),
                bitmapInfo: CGImageAlphaInfo.none.rawValue)
        else {
            clip(to: CGRect.null)
            return
        }
        mask.translateBy(x: -pixels.minX, y: -pixels.minY)
        mask.setFillColor(gray: 1, alpha: 1)
        for shape in shapes {
            mask.addPath(shape.path.cg.placed(under: shape.transform.cg.concatenating(root).concatenating(device)))
            mask.fillPath(using: shape.rule.cg)
        }
        guard let image = mask.makeImage() else {
            clip(to: CGRect.null)
            return
        }
        // `clip(to:mask:)` places the mask in user space, so step into device
        // space for the call and back out — without save/restore, which would
        // also undo the clip.
        concatenate(device.inverted())
        clip(to: pixels, mask: image)
        concatenate(device)
    }

    // MARK: Images

    private func draw(_ op: ImageOp, root: CGAffineTransform) {
        guard let source = CGImageSourceCreateWithData(op.data as CFData, nil),
            let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
        else { return }
        let rect = CGRect(x: 0, y: 0, width: CGFloat(op.width), height: CGFloat(op.height))
        drawFlipped(image, in: rect, under: op.transform.cg.concatenating(root), smooth: op.smooth)
    }

    private func draw(_ op: RasterOp, root: CGAffineTransform) {
        guard let image = op.cgImage else { return }
        let rect = CGRect(
            x: CGFloat(op.rect.x), y: CGFloat(op.rect.y),
            width: CGFloat(op.rect.width), height: CGFloat(op.rect.height))
        // Pixels were rendered for this context's scale, so resampling is
        // meant to be a no-op; `.high` keeps them right when it is not.
        drawFlipped(image, in: rect, under: root, smooth: true)
    }

    /// Draw `image` into `rect` of a y-down space. CoreGraphics draws images
    /// y-up, so the space is flipped across the rect for the call.
    private func drawFlipped(_ image: CGImage, in rect: CGRect, under transform: CGAffineTransform, smooth: Bool) {
        saveGState()
        defer { restoreGState() }
        concatenate(transform)
        translateBy(x: 0, y: rect.minY * 2 + rect.height)
        scaleBy(x: 1, y: -1)
        interpolationQuality = smooth ? .high : .none
        draw(image, in: rect)
    }
}

// MARK: - Type bridges

extension CGPath {
    /// A copy of the path with `transform` applied to every point.
    fileprivate func placed(under transform: CGAffineTransform) -> CGPath {
        var transform = transform
        return copy(using: &transform) ?? self
    }
}

extension Path {
    /// The path as a `CGPath`, built once per op.
    var cg: CGPath {
        let path = CGMutablePath()
        for verb in verbs {
            switch verb {
            case .moveTo(let x, let y):
                path.move(to: CGPoint(x: CGFloat(x), y: CGFloat(y)))
            case .lineTo(let x, let y):
                path.addLine(to: CGPoint(x: CGFloat(x), y: CGFloat(y)))
            case .quadTo(let x1, let y1, let x, let y):
                path.addQuadCurve(
                    to: CGPoint(x: CGFloat(x), y: CGFloat(y)),
                    control: CGPoint(x: CGFloat(x1), y: CGFloat(y1)))
            case .cubicTo(let x1, let y1, let x2, let y2, let x, let y):
                path.addCurve(
                    to: CGPoint(x: CGFloat(x), y: CGFloat(y)),
                    control1: CGPoint(x: CGFloat(x1), y: CGFloat(y1)),
                    control2: CGPoint(x: CGFloat(x2), y: CGFloat(y2)))
            case .close:
                path.closeSubpath()
            }
        }
        return path
    }
}

extension Transform {
    /// tiny-skia's `(sx, kx, ky, sy, tx, ty)` is CoreGraphics' `(a, c, b, d, tx, ty)`.
    var cg: CGAffineTransform {
        CGAffineTransform(
            a: CGFloat(sx), b: CGFloat(ky), c: CGFloat(kx), d: CGFloat(sy),
            tx: CGFloat(tx), ty: CGFloat(ty))
    }
}

extension Color {
    func cg(alpha: Float) -> CGColor {
        CGColor(
            colorSpace: CGColorSpace(name: CGColorSpace.sRGB)!,
            components: [
                CGFloat(red) / 255, CGFloat(green) / 255, CGFloat(blue) / 255,
                CGFloat(max(0, min(1, alpha))),
            ])!
    }
}

extension CGGradient {
    static func make(_ stops: [Stop]) -> CGGradient? {
        guard stops.count >= 2 else { return nil }
        let colors = stops.map { $0.color.cg(alpha: $0.opacity) } as CFArray
        let locations = stops.map { CGFloat($0.offset) }
        return CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB)!, colors: colors, locations: locations)
    }
}

extension RasterOp {
    /// The pixels as an image: premultiplied RGBA, 8 bits a channel, sRGB.
    var cgImage: CGImage? {
        let width = Int(self.width), height = Int(self.height)
        guard width > 0, height > 0, rgba.count == width * height * 4,
            let provider = CGDataProvider(data: rgba as CFData)
        else { return nil }
        return CGImage(
            width: width, height: height,
            bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue)
                .union(.byteOrder32Big),
            provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }
}

extension FillRule {
    var cg: CGPathFillRule {
        switch self {
        case .nonZero: return .winding
        case .evenOdd: return .evenOdd
        }
    }
}

extension LineCap {
    var cg: CGLineCap {
        switch self {
        case .butt: return .butt
        case .round: return .round
        case .square: return .square
        }
    }
}

extension LineJoin {
    /// CoreGraphics has no clipped miter; plain miter is what resvg's own
    /// backend substitutes too.
    var cg: CGLineJoin {
        switch self {
        case .miter, .miterClip: return .miter
        case .round: return .round
        case .bevel: return .bevel
        }
    }
}

extension BlendMode {
    var cg: CGBlendMode {
        switch self {
        case .normal: return .normal
        case .multiply: return .multiply
        case .screen: return .screen
        case .overlay: return .overlay
        case .darken: return .darken
        case .lighten: return .lighten
        case .colorDodge: return .colorDodge
        case .colorBurn: return .colorBurn
        case .hardLight: return .hardLight
        case .softLight: return .softLight
        case .difference: return .difference
        case .exclusion: return .exclusion
        case .hue: return .hue
        case .saturation: return .saturation
        case .color: return .color
        case .luminosity: return .luminosity
        }
    }
}

extension Array where Element == (CGPath, CGAffineTransform) {
    /// Whether no two paths' transformed bounding boxes overlap — when
    /// concatenating them is a true union.
    fileprivate var disjoint: Bool {
        let boxes = map { $0.0.boundingBoxOfPath.applying($0.1) }
        for (i, a) in boxes.enumerated() {
            for b in boxes[(i + 1)...] where a.intersects(b) { return false }
        }
        return true
    }
}
