import QuartzCore
import SwiftUI
import UIKit

/// The scene walking past, rendered live on its own thread at a fraction of the screen's size (as
/// small as it needs to keep up, aiming for 20-30 frames a second) and scaled up by Core Animation.
/// It stops while the app is in the background: iOS does not let an app use the GPU there.
final class PreviewUIView: UIView {
    private let lock = NSLock()
    private var pendingScene: String?
    private var reopenWanted = false
    private var running = false
    private var pixelSize = CGSize.zero
    private let t0 = CACurrentMediaTime()
    private var nowValue: (Float, Float) = (0, 0)
    /// Told about twice a second what is drawing, at what size and how fast.
    var onStats: ((Bool, Int, Int, Double) -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .black
        layer.contentsGravity = .resizeAspectFill
        layer.masksToBounds = true
        let nc = NotificationCenter.default
        nc.addObserver(self, selector: #selector(pause), name: UIApplication.willResignActiveNotification, object: nil)
        nc.addObserver(self, selector: #selector(resume), name: UIApplication.didBecomeActiveNotification, object: nil)
    }

    required init?(coder: NSCoder) { fatalError("not from a storyboard") }

    deinit { NotificationCenter.default.removeObserver(self) }

    /// Where the walk is now: metres along and seconds into the loop.
    var now: (distance: Float, time: Float) {
        lock.lock(); defer { lock.unlock() }
        return nowValue
    }

    /// While editing: the whole frame, as the phone's screen shows it, fitted into the view.
    func setFit(_ on: Bool) {
        lock.lock(); fit = on; lock.unlock()
        layer.contentsGravity = on ? .resizeAspect : .resizeAspectFill
    }
    private var fit = false

    func setScene(_ json: String) {
        lock.lock(); pendingScene = json; lock.unlock()
        layer.magnificationFilter = Look.pixelated(json) ? .nearest : .linear
    }

    /// Open the scene again (after the GPU is switched on or off).
    func reopen(_ json: String) {
        lock.lock(); pendingScene = json; reopenWanted = true; lock.unlock()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        let s = window?.screen.scale ?? UIScreen.main.scale
        lock.lock(); pixelSize = CGSize(width: bounds.width * s, height: bounds.height * s); lock.unlock()
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil { resume() } else { pause() }
    }

    @objc func resume() {
        lock.lock()
        let start = !running
        running = true
        lock.unlock()
        if start {
            let t = Thread { [weak self] in self?.loop() }
            t.name = "pathforge-preview"
            t.qualityOfService = .userInteractive
            t.start()
        }
    }

    @objc func pause() {
        lock.lock(); running = false; lock.unlock()
    }

    private var isRunning: Bool { lock.lock(); defer { lock.unlock() }; return running }

    private func loop() {
        var scene: PathScene?
        var loopLen: Float = 24, loopSecs: Float = 10
        // Render at 1/div of the screen's width, adapting to how fast this phone draws the scene.
        var div = 3
        var avg = 30.0
        var frames = 0
        var first = true
        var statsAt = CACurrentMediaTime()
        while isRunning {
            let frameStart = CACurrentMediaTime()
            lock.lock()
            let next = pendingScene
            let again = reopenWanted
            pendingScene = nil
            reopenWanted = false
            var size = pixelSize
            if fit {
                // The screen's shape, at the view's width.
                let b = UIScreen.main.nativeBounds
                size = CGSize(width: size.width, height: size.width * max(b.width, b.height) / min(b.width, b.height))
            }
            lock.unlock()
            if let json = next {
                if again { scene = nil; first = true }
                if let s = scene, s.setScene(json) {
                } else {
                    scene = Gpu.open(json)
                }
                if let s = scene { (loopLen, loopSecs) = s.loop }
            }
            let w = Int(size.width), h = Int(size.height)
            guard let s = scene, w > 0, h > 0 else { Thread.sleep(forTimeInterval: 0.05); continue }
            let rw = max(64, w / div), rh = max(64, Int((Double(h) * Double(rw) / Double(w)).rounded()))
            let t = Float((CACurrentMediaTime() - t0).truncatingRemainder(dividingBy: Double(loopSecs)))
            let d = t / loopSecs * loopLen
            let a = CACurrentMediaTime()
            let img = s.image(distance: d, time: t, width: rw, height: rh)
            avg = avg * 0.8 + (CACurrentMediaTime() - a) * 1000 * 0.2
            if let img = img {
                lock.lock(); nowValue = (d, t); lock.unlock()
                DispatchQueue.main.async { [weak self] in self?.layer.contents = img }
                if first { Gpu.rendered(s); first = false }
                frames += 1
            }
            let now = CACurrentMediaTime()
            if now - statsAt >= 0.5 {
                let fps = Double(frames) / (now - statsAt)
                let gpu = s.onGPU
                DispatchQueue.main.async { [weak self] in self?.onStats?(gpu, rw, rh, fps) }
                frames = 0
                statsAt = now
            }
            // As much of the screen's size as keeps up: half at best, a tenth at worst.
            if avg > 45 && div < 10 { div += 1; avg = 30 } else if avg < 16 && div > 2 { div -= 1; avg = 30 }
            let spare = 1.0 / 30 - (CACurrentMediaTime() - frameStart)
            if spare > 0 { Thread.sleep(forTimeInterval: spare) }
        }
    }
}

struct PreviewView: UIViewRepresentable {
    let model: AppModel

    func makeUIView(context: Context) -> PreviewUIView {
        let v = PreviewUIView(frame: .zero)
        v.onStats = { gpu, w, h, fps in model.showStats(gpu: gpu, width: w, height: h, fps: fps) }
        model.preview = v
        v.setScene(model.scene)
        return v
    }

    func updateUIView(_ v: PreviewUIView, context: Context) {}
}
