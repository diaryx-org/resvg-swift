// Draws real SVGs through the Rust binding into bitmap contexts and reads the
// pixels back. The Rust side already proves the display list matches resvg;
// these prove the CoreGraphics replay of it puts the right colour in the right
// place — including the two things a CoreGraphics backend is most likely to get
// wrong, the y direction of images and the composition of group opacity.

import CoreGraphics
import Foundation
import ResvgFFI
import XCTest

@testable import ResvgCoreGraphics

final class ReplayTests: XCTestCase {
    /// A y-down RGBA canvas with a pixel reader.
    struct Canvas {
        let context: CGContext
        let width: Int
        let height: Int

        init(width: Int, height: Int) {
            self.width = width
            self.height = height
            context = CGContext(
                data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue)!
            // Bitmap contexts are y-up; SVG is y-down.
            context.translateBy(x: 0, y: CGFloat(height))
            context.scaleBy(x: 1, y: -1)
        }

        /// Premultiplied RGBA at (x, y), y from the top.
        func pixel(_ x: Int, _ y: Int) -> (r: UInt8, g: UInt8, b: UInt8, a: UInt8) {
            let data = context.data!.assumingMemoryBound(to: UInt8.self)
            let i = y * context.bytesPerRow + x * 4
            return (data[i], data[i + 1], data[i + 2], data[i + 3])
        }
    }

    private func picture(_ svg: String) throws -> SVGPicture {
        try SVGPicture(data: Data(svg.utf8), options: ParseOptions(loadSystemFonts: false))
    }

    private func draw(_ svg: String, size: Int = 40) throws -> Canvas {
        let canvas = Canvas(width: size, height: size)
        try picture(svg).draw(
            in: canvas.context, rect: CGRect(x: 0, y: 0, width: size, height: size))
        return canvas
    }

    func testSolidFillLandsWhereTheRectIs() throws {
        let canvas = try draw(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <rect x="0" y="0" width="20" height="20" fill="#ff0000"/>
            </svg>
            """)
        let inside = canvas.pixel(5, 5)
        XCTAssertEqual(inside.r, 255)
        XCTAssertEqual(inside.g, 0)
        XCTAssertEqual(inside.a, 255)
        // Top-left in SVG is top-left on the canvas: the opposite corner is empty.
        XCTAssertEqual(canvas.pixel(35, 35).a, 0)
        // And it is the *top*, not the bottom — the y-direction check.
        XCTAssertEqual(canvas.pixel(5, 35).a, 0)
    }

    func testStrokeIsInLocalUnitsAndScalesWithTheTransform() throws {
        // A 2-unit stroke on a 20-unit canvas drawn into 40 pixels is 4 pixels
        // wide: pixel 2 (inside the stroke band) is painted, pixel 6 is not.
        let canvas = try draw(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20">
              <rect x="1" y="1" width="18" height="18" fill="none" stroke="#0000ff" stroke-width="2"/>
            </svg>
            """)
        XCTAssertEqual(canvas.pixel(2, 10).b, 255)
        XCTAssertEqual(canvas.pixel(6, 10).a, 0)
    }

    func testGroupOpacityCompositesOnce() throws {
        // Two overlapping opaque rects in a half-opacity group: the overlap must
        // be one 50% blue layer, not blue-over-red at 50% each (which would
        // leave red showing through).
        let canvas = try draw(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <g opacity="0.5">
                <rect width="40" height="40" fill="#ff0000"/>
                <rect width="40" height="40" fill="#0000ff"/>
              </g>
            </svg>
            """)
        let p = canvas.pixel(20, 20)
        XCTAssertEqual(Int(p.a), 128, accuracy: 2)
        XCTAssertEqual(Int(p.b), 128, accuracy: 2)
        XCTAssertEqual(p.r, 0, "red must not leak through the layer")
    }

    func testClipPathCutsTheFill() throws {
        let canvas = try draw(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <clipPath id="c"><rect x="0" y="0" width="20" height="40"/></clipPath>
              <rect width="40" height="40" fill="#00ff00" clip-path="url(#c)"/>
            </svg>
            """)
        XCTAssertEqual(canvas.pixel(10, 20).g, 255)
        XCTAssertEqual(canvas.pixel(30, 20).a, 0)
    }

    func testClipOfOverlappingShapesIsAUnion() throws {
        // Two overlapping rects in one clipPath. Concatenated as one path their
        // overlap would cancel under even-odd; the union must keep it.
        let canvas = try draw(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <clipPath id="c" clip-rule="evenodd">
                <rect x="0" y="0" width="30" height="40"/>
                <rect x="10" y="0" width="30" height="40"/>
              </clipPath>
              <rect width="40" height="40" fill="#00ff00" clip-path="url(#c)"/>
            </svg>
            """)
        XCTAssertEqual(canvas.pixel(20, 20).g, 255, "the overlap is inside the union")
        XCTAssertEqual(canvas.pixel(5, 20).g, 255)
        XCTAssertEqual(canvas.pixel(35, 20).g, 255)
    }

    func testLinearGradientRunsTheRightWay() throws {
        let canvas = try draw(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
                <stop offset="0" stop-color="#ff0000"/>
                <stop offset="1" stop-color="#0000ff"/>
              </linearGradient>
              <rect width="40" height="40" fill="url(#g)"/>
            </svg>
            """)
        let left = canvas.pixel(1, 20), right = canvas.pixel(38, 20)
        XCTAssertGreaterThan(left.r, 220)
        XCTAssertGreaterThan(right.b, 220)
    }

    func testFilteredGroupArrivesAsRasterAndDrawsUpright() throws {
        // A blurred blue square in the top-left. The display list carries it as
        // pixels; they must land top-left too, not flipped to the bottom.
        let svg = """
            <svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
              <filter id="b"><feGaussianBlur stdDeviation="1"/></filter>
              <rect x="2" y="2" width="16" height="16" fill="#0000ff" filter="url(#b)"/>
            </svg>
            """
        let list = try picture(svg).displayList(rasterScale: 1)
        XCTAssertEqual(list.ops.count, 1)
        guard case .raster = list.ops[0] else { return XCTFail("expected a raster op") }
        let canvas = try draw(svg)
        XCTAssertGreaterThan(canvas.pixel(10, 10).b, 200)
        XCTAssertEqual(canvas.pixel(10, 32).a, 0)
    }

    func testAspectFitCentres() throws {
        // A 20×10 red picture into a 40×40 rect: 40×20, centred vertically.
        let canvas = try draw(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
              <rect width="20" height="10" fill="#ff0000"/>
            </svg>
            """)
        XCTAssertEqual(canvas.pixel(20, 20).r, 255)
        XCTAssertEqual(canvas.pixel(20, 5).a, 0)
        XCTAssertEqual(canvas.pixel(20, 35).a, 0)
    }

    func testDrawingIsClippedToTheCanvas() throws {
        // A 20×10 picture whose rect runs far past its own width, drawn into
        // 40×40: the overflow must stop at the fitted picture's edge (rows
        // 10..<30), not paint the rest of the context.
        let canvas = try draw(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="10">
              <rect x="-100" y="-100" width="300" height="300" fill="#ff0000"/>
            </svg>
            """)
        XCTAssertEqual(canvas.pixel(20, 20).r, 255)
        XCTAssertEqual(canvas.pixel(20, 5).a, 0)
        XCTAssertEqual(canvas.pixel(20, 35).a, 0)
    }

    func testParseErrorIsThrown() {
        XCTAssertThrowsError(try picture("not an svg"))
    }
}

extension ReplayTests {
    /// A group's translate is in canvas units and must be scaled by the fit,
    /// not applied after it: a 20-unit canvas drawn into 40 pixels puts a
    /// square translated to (10, 10) at pixel (20, 20), not (10, 10).
    func testAnOpTransformIsScaledByTheFit() throws {
        let canvas = Canvas(width: 40, height: 40)
        try picture(
            """
            <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20">
              <g transform="translate(10 10)"><rect width="5" height="5" fill="#ff0000"/></g>
            </svg>
            """
        ).draw(in: canvas.context, rect: CGRect(x: 0, y: 0, width: 40, height: 40))
        XCTAssertEqual(canvas.pixel(25, 25).r, 255, "inside the square, scaled and translated")
        XCTAssertEqual(canvas.pixel(15, 15).a, 0, "where the square would be if the translate were not scaled")
    }
}
