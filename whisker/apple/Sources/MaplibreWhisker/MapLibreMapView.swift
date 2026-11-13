// The map's view: a view whose layer is a `CAMetalLayer`, which the Rust
// map renders into on a thread of its own. The view hands the map its props,
// size, input and commands through the crate's C functions (`src/ffi.rs`),
// runs its frames on a display link while the map wants them, posts the
// events each frame returns, and moves its markers' nodes to their places.

import Foundation
import QuartzCore
#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif
import WhiskerModule

// The crate's C functions, which the app's Rust library exports.
@_silgen_name("maplibre_whisker_new")
private func maplibre_whisker_new(
    _ layer: UnsafeMutableRawPointer?,
    _ wake: (@convention(c) (UnsafeMutableRawPointer?) -> Void)?,
    _ view: UnsafeMutableRawPointer?
) -> UInt64
@_silgen_name("maplibre_whisker_drop")
private func maplibre_whisker_drop(_ map: UInt64)
@_silgen_name("maplibre_whisker_set_prop")
private func maplibre_whisker_set_prop(_ map: UInt64, _ name: UnsafePointer<CChar>, _ value: UnsafePointer<CChar>)
@_silgen_name("maplibre_whisker_command")
private func maplibre_whisker_command(_ map: UInt64, _ json: UnsafePointer<CChar>)
@_silgen_name("maplibre_whisker_resize")
private func maplibre_whisker_resize(_ map: UInt64, _ width: Double, _ height: Double, _ scale: Double)
@_silgen_name("maplibre_whisker_pointer")
private func maplibre_whisker_pointer(
    _ map: UInt64, _ id: UInt32, _ phase: Int32, _ x: Double, _ y: Double, _ button: Int32, _ touch: Bool,
    _ modifiers: UInt32
)
@_silgen_name("maplibre_whisker_wheel")
private func maplibre_whisker_wheel(
    _ map: UInt64, _ x: Double, _ y: Double, _ delta: Double, _ precise: Bool, _ modifiers: UInt32
)
@_silgen_name("maplibre_whisker_pinch")
private func maplibre_whisker_pinch(
    _ map: UInt64, _ x: Double, _ y: Double, _ scale: Double, _ rotation: Double, _ phase: Int32
)
@_silgen_name("maplibre_whisker_frame")
private func maplibre_whisker_frame(_ map: UInt64, _ again: UnsafeMutablePointer<Bool>?) -> UnsafeMutablePointer<CChar>?
@_silgen_name("maplibre_whisker_place")
private func maplibre_whisker_place(
    _ map: UInt64, _ marker: UnsafePointer<CChar>, _ x: Double, _ y: Double, _ width: Double, _ height: Double,
    _ out: UnsafeMutablePointer<Double>?
) -> Int32
@_silgen_name("maplibre_whisker_free")
private func maplibre_whisker_free(_ text: UnsafeMutablePointer<CChar>?)

/// The display link's target, which holds the view weakly, since a display
/// link retains its target.
private final class FrameTarget: NSObject {
    weak var view: MapLibreLayerView?

    init(_ view: MapLibreLayerView) {
        self.view = view
    }

    @objc func step(_ link: CADisplayLink) {
        view?.frame(link)
    }
}

#if canImport(UIKit)
public typealias MapLibrePlatformView = UIView
#else
public typealias MapLibrePlatformView = NSView
#endif

/// The view the map renders into, which takes the touches, the mouse and
/// the trackpad.
public final class MapLibreLayerView: MapLibrePlatformView {
    private var map: UInt64 = 0
    private var link: CADisplayLink?
    /// Posts an event of the map, by its name, with its record as JSON.
    var post: ((String, String) -> Void)?

    #if canImport(UIKit)
    public override class var layerClass: AnyClass { CAMetalLayer.self }
    private var metalLayer: CAMetalLayer { layer as! CAMetalLayer }
    private var touchIds: [ObjectIdentifier: UInt32] = [:]
    private var nextTouch: UInt32 = 1
    #else
    private let metalLayer = CAMetalLayer()
    public override func makeBackingLayer() -> CALayer { metalLayer }
    public override var wantsUpdateLayer: Bool { true }
    public override var isFlipped: Bool { true }
    public override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    #endif

    public override init(frame: CGRect) {
        super.init(frame: frame)
        #if canImport(UIKit)
        isMultipleTouchEnabled = true
        #else
        wantsLayer = true
        #endif
        metalLayer.isOpaque = true
        let wake: @convention(c) (UnsafeMutableRawPointer?) -> Void = { view in
            guard let view else { return }
            Unmanaged<MapLibreLayerView>.fromOpaque(view).takeUnretainedValue().resume()
        }
        // The map calls `wake` with this view, which drops the map before it goes.
        map = maplibre_whisker_new(
            Unmanaged.passUnretained(metalLayer).toOpaque(),
            wake,
            Unmanaged.passUnretained(self).toOpaque()
        )
    }

    public required init?(coder: NSCoder) { nil }

    deinit {
        link?.invalidate()
        maplibre_whisker_drop(map)
    }

    func setProp(_ name: String, _ value: String) {
        maplibre_whisker_set_prop(map, name, value)
    }

    func command(_ json: String) {
        maplibre_whisker_command(map, json)
    }

    /// Runs frames until the map wants none.
    func resume() {
        if link == nil {
            #if canImport(UIKit)
            let link = CADisplayLink(target: FrameTarget(self), selector: #selector(FrameTarget.step(_:)))
            #else
            let link = displayLink(target: FrameTarget(self), selector: #selector(FrameTarget.step(_:)))
            #endif
            link.add(to: .main, forMode: .common)
            self.link = link
        }
        link?.isPaused = false
    }

    fileprivate func frame(_ link: CADisplayLink) {
        var again = false
        if let text = maplibre_whisker_frame(map, &again) {
            let posts = String(cString: text)
            maplibre_whisker_free(text)
            let list = (try? JSONSerialization.jsonObject(with: Data(posts.utf8))) as? [[String: Any]] ?? []
            for event in list {
                if let name = event["name"] as? String, let record = event["record"] as? String {
                    post?(name, record)
                }
            }
        }
        if !again {
            link.isPaused = true
        }
        placeMarkers()
    }

    /// Moves the nodes of the map's markers to where the content's markers
    /// put them, on the map as its last frame drew it.
    func placeMarkers() {
        guard let node = superview?.superview else { return }
        for marker in markers(in: node) {
            place(marker)
        }
    }

    /// The markers among a node's children, but not those of a map inside it.
    private func markers(in view: MapLibrePlatformView) -> [MapLibreMarkerView] {
        view.subviews.flatMap { child -> [MapLibreMarkerView] in
            if child is MapLibreMapView { return [] }
            if child.subviews.contains(where: { $0 is MapLibreMapView }) { return [] }
            if let marker = child as? MapLibreMarkerView { return [marker] }
            return markers(in: child)
        }
    }

    private func place(_ marker: MapLibreMarkerView) {
        guard let node = marker.superview, let parent = node.superview else { return }
        #if canImport(UIKit)
        let origin = node.layer.position
        #else
        // A node that isn't where the map moved it is where the layout put it.
        if marker.placedOrigin != node.frame.origin { marker.layoutOrigin = node.frame.origin }
        let origin = marker.layoutOrigin
        #endif
        let at = convert(origin, from: parent)
        var h = [Double](repeating: 0, count: 9)
        let placed = maplibre_whisker_place(
            map, marker.marker, Double(at.x), Double(at.y), Double(node.bounds.width), Double(node.bounds.height), &h
        )
        if (placed == 2) != marker.hiddenByMap {
            marker.hiddenByMap = placed == 2
            node.isHidden = marker.hiddenByMap
        }
        #if canImport(UIKit)
        if placed == 1 {
            // The homography maps the node's points to the map's, and the layer
            // turns about its zero anchor at the layout's origin.
            let (x, y) = (Double(at.x), Double(at.y))
            var transform = CATransform3DIdentity
            transform.m11 = CGFloat(h[0] - x * h[6])
            transform.m21 = CGFloat(h[1] - x * h[7])
            transform.m41 = CGFloat(h[2] - x * h[8])
            transform.m12 = CGFloat(h[3] - y * h[6])
            transform.m22 = CGFloat(h[4] - y * h[7])
            transform.m42 = CGFloat(h[5] - y * h[8])
            transform.m14 = CGFloat(h[6])
            transform.m24 = CGFloat(h[7])
            transform.m44 = CGFloat(h[8])
            node.layer.transform = transform
            marker.placed = true
        } else if marker.placed {
            node.layer.transform = CATransform3DIdentity
            marker.placed = false
        }
        #else
        // AppKit's node layers keep Whisker's own transform, so a marker on
        // macOS moves to its place upright.
        let moved: CGPoint? = placed == 1
            ? CGPoint(x: origin.x + CGFloat(h[2]) - at.x, y: origin.y + CGFloat(h[5]) - at.y) : nil
        if let moved, moved != node.frame.origin {
            node.setFrameOrigin(moved)
        } else if moved == nil, marker.placedOrigin != nil {
            node.setFrameOrigin(marker.layoutOrigin)
        }
        marker.placedOrigin = moved
        #endif
    }

    private var scale: CGFloat {
        #if canImport(UIKit)
        window?.screen.scale ?? traitCollection.displayScale
        #else
        window?.backingScaleFactor ?? 2
        #endif
    }

    private func resized() {
        metalLayer.contentsScale = scale
        maplibre_whisker_resize(map, Double(bounds.width), Double(bounds.height), Double(scale))
    }

    #if canImport(UIKit)
    public override func layoutSubviews() {
        super.layoutSubviews()
        resized()
    }

    public override func didMoveToWindow() {
        super.didMoveToWindow()
        resized()
    }

    private func send(_ touches: Set<UITouch>, _ phase: Int32) {
        for touch in touches {
            let key = ObjectIdentifier(touch)
            let id: UInt32
            if phase == 0 {
                id = nextTouch
                nextTouch &+= 1
                touchIds[key] = id
            } else {
                guard let known = touchIds[key] else { continue }
                id = known
            }
            if phase >= 2 {
                touchIds[key] = nil
            }
            let at = touch.location(in: self)
            maplibre_whisker_pointer(map, id, phase, Double(at.x), Double(at.y), 0, true, 0)
        }
    }

    public override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, 0) }
    public override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, 1) }
    public override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, 2) }
    public override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) { send(touches, 3) }
    #else
    public override func layout() {
        super.layout()
        resized()
    }

    public override func viewDidChangeBackingProperties() {
        super.viewDidChangeBackingProperties()
        resized()
    }

    public override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        resized()
    }

    private func modifiers(_ event: NSEvent) -> UInt32 {
        let flags = event.modifierFlags
        return (flags.contains(.shift) ? 1 : 0) | (flags.contains(.control) ? 2 : 0)
            | (flags.contains(.option) ? 4 : 0) | (flags.contains(.command) ? 8 : 0)
    }

    private func press(_ event: NSEvent, _ phase: Int32, _ button: Int32) {
        let at = convert(event.locationInWindow, from: nil)
        maplibre_whisker_pointer(map, 0, phase, Double(at.x), Double(at.y), button, false, modifiers(event))
    }

    public override func mouseDown(with event: NSEvent) { press(event, 0, 0) }
    public override func mouseDragged(with event: NSEvent) { press(event, 1, 0) }
    public override func mouseUp(with event: NSEvent) { press(event, 2, 0) }
    public override func rightMouseDown(with event: NSEvent) { press(event, 0, 2) }
    public override func rightMouseDragged(with event: NSEvent) { press(event, 1, 2) }
    public override func rightMouseUp(with event: NSEvent) { press(event, 2, 2) }
    public override func otherMouseDown(with event: NSEvent) { press(event, 0, 1) }
    public override func otherMouseDragged(with event: NSEvent) { press(event, 1, 1) }
    public override func otherMouseUp(with event: NSEvent) { press(event, 2, 1) }

    public override func scrollWheel(with event: NSEvent) {
        let at = convert(event.locationInWindow, from: nil)
        // The web's sign, which the map's gestures take: down is positive.
        maplibre_whisker_wheel(
            map, Double(at.x), Double(at.y), -Double(event.scrollingDeltaY), event.hasPreciseScrollingDeltas,
            modifiers(event)
        )
    }

    private func pinchPhase(_ event: NSEvent) -> Int32 {
        if event.phase.contains(.began) { return 0 }
        if event.phase.contains(.ended) { return 2 }
        if event.phase.contains(.cancelled) { return 3 }
        return 1
    }

    public override func magnify(with event: NSEvent) {
        let at = convert(event.locationInWindow, from: nil)
        maplibre_whisker_pinch(map, Double(at.x), Double(at.y), 1 + Double(event.magnification), 0, pinchPhase(event))
    }

    public override func rotate(with event: NSEvent) {
        let at = convert(event.locationInWindow, from: nil)
        // AppKit turns counterclockwise, the map's gestures clockwise.
        maplibre_whisker_pinch(map, Double(at.x), Double(at.y), 1, -Double(event.rotation), pinchPhase(event))
    }
    #endif
}

/// The map as Whisker's module view.
@objc(MapLibreMapView)
public final class MapLibreMapView: WhiskerUI<MapLibreLayerView> {
    @objc public override func createView() -> MapLibreLayerView {
        let view = MapLibreLayerView(frame: .zero)
        view.post = { [weak self] name, record in
            self?.emitWhiskerEvent(name, detail: .map(["record": .string(record)]))
        }
        return view
    }

    func setProp(_ name: String, _ value: String) {
        view().setProp(name, value)
    }

    func command(_ json: String) {
        view().command(json)
    }
}
