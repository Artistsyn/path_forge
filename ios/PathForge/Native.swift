import CoreGraphics
import Foundation

/// PathForge's renderer (libpathforge.a, mobile/). Scenes are JSON.
enum Native {
    /// A string the library returned, freed.
    static func take(_ p: UnsafeMutablePointer<CChar>?) -> String? {
        guard let p = p else { return nil }
        defer { pfm_free(p) }
        return String(cString: p)
    }

    static func presetNames() -> [String] {
        guard let s = take(pfm_preset_names()), let d = s.data(using: .utf8),
              let a = (try? JSONSerialization.jsonObject(with: d)) as? [String] else { return [] }
        return a
    }

    static func presetScene(_ name: String) -> String? { take(pfm_preset_scene(name)) }
    static func normalize(_ json: String) -> String? { take(pfm_normalize(json)) }
    static var lastError: String { take(pfm_last_error()) ?? "" }
    /// Start the GPU renderer: its name, or "" where there is none.
    static func gpuInit() -> String { take(pfm_gpu_init()) ?? "" }
}

/// One scene being drawn, used from one thread at a time.
final class PathScene {
    private let handle: OpaquePointer

    init?(json: String, gpu: Bool) {
        guard let h = pfm_open(json, "", gpu) else { return nil }
        handle = h
    }

    deinit { pfm_close(handle) }

    @discardableResult
    func setScene(_ json: String) -> Bool { pfm_set_scene(handle, json) }

    /// Metres before the view repeats, and seconds before everything that moves repeats.
    var loop: (length: Float, seconds: Float) {
        var v = [Float](repeating: 0, count: 5)
        _ = v.withUnsafeMutableBufferPointer { pfm_info(handle, $0.baseAddress) }
        return (max(v[0], 1), max(v[1], 0.1))
    }

    var onGPU: Bool { pfm_on_gpu(handle) }

    /// The view `distance` metres along at `time` seconds, as opaque RGBA rows.
    func render(distance: Float, time: Float, width: Int, height: Int) -> Data? {
        // Scenes whose day and night follow the clock take the phone's time of day.
        let now = Calendar.current.dateComponents([.hour, .minute, .second], from: Date())
        pfm_set_clock(Float(now.hour ?? 12) + Float(now.minute ?? 0) / 60 + Float(now.second ?? 0) / 3600)
        let n = width * height * 4
        var data = Data(count: n)
        let ok = data.withUnsafeMutableBytes { raw -> Bool in
            guard let base = raw.baseAddress?.assumingMemoryBound(to: UInt8.self) else { return false }
            return pfm_render(handle, distance, time, UInt32(width), UInt32(height), base, n)
        }
        return ok ? data : nil
    }

    func image(distance: Float, time: Float, width: Int, height: Int) -> CGImage? {
        guard let data = render(distance: distance, time: time, width: width, height: height) else { return nil }
        return PathScene.cgImage(data, width: width, height: height)
    }

    static func cgImage(_ data: Data, width: Int, height: Int) -> CGImage? {
        guard let provider = CGDataProvider(data: data as CFData),
              let space = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        return CGImage(width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4, space: space,
                       bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue), provider: provider,
                       decode: nil, shouldInterpolate: true, intent: .defaultIntent)
    }
}

/// Whether to draw on the GPU, and a guard against a GPU driver that brings the app down (which no
/// code can catch): the first GPU frame of a run is announced before and confirmed after, so a run
/// that died in between is found on the next start, which then stays on the CPU.
enum Gpu {
    private static let defaults = UserDefaults.standard
    private static var proven = false
    private static let lock = NSLock()

    static var wanted: Bool {
        get { defaults.object(forKey: "gpu.on") as? Bool ?? true }
        set { defaults.set(newValue, forKey: "gpu.on"); defaults.set(false, forKey: "gpu.pending") }
    }

    /// True (once) if the last run died while starting the GPU: the GPU is then switched off.
    static func crashedLastTime() -> Bool {
        guard defaults.bool(forKey: "gpu.pending") else { return false }
        defaults.set(false, forKey: "gpu.pending")
        defaults.set(false, forKey: "gpu.on")
        return true
    }

    /// Open a scene on the GPU if it is wanted.
    static func open(_ json: String) -> PathScene? {
        let gpu = wanted
        lock.lock()
        let unproven = !proven
        lock.unlock()
        if gpu && unproven { defaults.set(true, forKey: "gpu.pending"); defaults.synchronize() }
        let s = PathScene(json: json, gpu: gpu)
        if gpu, let s = s, !s.onGPU { settled() }
        return s
    }

    /// After a frame: once one GPU frame has been drawn, the GPU is known to work.
    static func rendered(_ s: PathScene) {
        lock.lock()
        let unproven = !proven
        lock.unlock()
        if unproven && s.onGPU { settled() }
    }

    private static func settled() {
        lock.lock()
        proven = true
        lock.unlock()
        defaults.set(false, forKey: "gpu.pending")
        defaults.synchronize()
    }
}
