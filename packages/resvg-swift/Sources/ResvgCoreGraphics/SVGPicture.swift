// A parsed SVG, ready to draw into any CGContext at any size.
//
// `SvgDocument` (the Rust object) is the expensive part — parsing, CSS, text
// layout — and lives as long as this does. The display list is asked for per
// scale and cached, because its one scale-dependent part (the rasterized
// fallback for filters and the like) has to be redone for a sharper screen
// and nothing else in it changes.

import CoreGraphics
import Foundation
import ResvgFFI

public final class SVGPicture {
    /// The parsed document.
    public let document: SvgDocument
    /// The document's own size, in user units — its `width`/`height` as usvg
    /// resolved them. The aspect ratio to fit by.
    public let size: CGSize

    private var cache: (scale: CGFloat, list: DisplayList)?

    /// Parse `data` — plain or gzip-compressed SVG.
    public init(data: Data, options: ParseOptions = ParseOptions()) throws {
        document = try SvgDocument(data: data, options: options)
        size = CGSize(width: CGFloat(document.width()), height: CGFloat(document.height()))
    }

    /// Parse the file at `url`; relative `href`s resolve against its directory.
    public convenience init(contentsOf url: URL, options: ParseOptions = ParseOptions()) throws {
        var options = options
        if options.resourcesDir == nil {
            options.resourcesDir = url.deletingLastPathComponent().path
        }
        try self.init(data: try Data(contentsOf: url), options: options)
    }

    /// The display list with rasterized parts at `rasterScale` device pixels
    /// per user unit. Cached for the last scale asked.
    public func displayList(rasterScale: CGFloat) -> DisplayList {
        if let cache, cache.scale == rasterScale { return cache.list }
        let list = document.displayList(rasterScale: Float(rasterScale))
        cache = (rasterScale, list)
        return list
    }

    /// Draw the picture fitted into `rect`, preserving aspect ratio and centred,
    /// in a **y-down** user space — a `UIView`, a flipped `NSView`, a PDF page
    /// after a flip. In a y-up context, flip first.
    ///
    /// `scale` is the context's device pixels per point (a view's
    /// `backingScaleFactor` / `contentScaleFactor`; `1` for a PDF), so anything
    /// rasterized comes out at the pixel grid it will be shown on. Vector ops
    /// do not care.
    ///
    /// Drawing is clipped to the picture's own canvas, as a browser clips an
    /// `<img>` to its viewport and resvg to its pixmap: a path that runs past
    /// the SVG's `width` stops at the edge rather than spilling into the page.
    public func draw(in context: CGContext, rect: CGRect, scale: CGFloat = 1) {
        guard size.width > 0, size.height > 0, rect.width > 0, rect.height > 0 else { return }
        let fit = min(rect.width / size.width, rect.height / size.height)
        let drawn = CGSize(width: size.width * fit, height: size.height * fit)
        let origin = CGPoint(
            x: rect.minX + (rect.width - drawn.width) / 2,
            y: rect.minY + (rect.height - drawn.height) / 2)
        let transform = CGAffineTransform(translationX: origin.x, y: origin.y).scaledBy(x: fit, y: fit)
        context.saveGState()
        defer { context.restoreGState() }
        context.clip(to: CGRect(origin: origin, size: drawn))
        context.draw(displayList(rasterScale: fit * scale), transform: transform)
    }
}
