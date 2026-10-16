import Foundation

/// The scene the phone is working on (a preset, one of yours, or a file from the desktop), kept
/// between runs. The quick controls change the scene itself, as the editor does, so what is saved
/// is what is shown.
struct Look {
    static let skies = ["As designed", "Golden", "Fiery", "Pastel", "Dusk"]
    static let times = ["As designed", "Real time", "Dawn", "Noon", "Dusk", "Night", "Day cycle"]
    /// The hour each of `times`' fixed times stands at.
    private static let hours: [Double] = [-1, -1, 6.4, 12, 18.1, 23, 5]
    static let defaultPreset = "Sunset Pier"

    var name: String
    /// Whether it is one of the scenes you saved under that name.
    var mine = false
    /// The scene, as JSON.
    var scene: String

    private static var file: URL { Library.support.appendingPathComponent("scene.json") }

    static func load() -> Look {
        let d = UserDefaults.standard
        if let saved = try? String(contentsOf: file, encoding: .utf8), !saved.isEmpty {
            return Look(name: d.string(forKey: "look.name") ?? defaultPreset, mine: d.bool(forKey: "look.mine"), scene: saved)
        }
        return Look(name: defaultPreset, mine: false, scene: Native.presetScene(defaultPreset) ?? "{}")
    }

    func save() {
        UserDefaults.standard.set(name, forKey: "look.name")
        UserDefaults.standard.set(mine, forKey: "look.mine")
        try? scene.write(to: Look.file, atomically: true, encoding: .utf8)
    }

    // ── Quick controls ───────────────────────────────────────────────────

    private func parsed() -> NSMutableDictionary {
        guard let d = scene.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d, options: [.mutableContainers])) as? NSMutableDictionary else { return NSMutableDictionary() }
        return o
    }

    private mutating func store(_ o: NSMutableDictionary) {
        if let d = try? JSONSerialization.data(withJSONObject: o), let s = String(data: d, encoding: .utf8) { scene = s }
    }

    private static func obj(_ parent: NSMutableDictionary, _ key: String) -> NSMutableDictionary {
        if let o = parent[key] as? NSMutableDictionary { return o }
        let o = NSMutableDictionary()
        parent[key] = o
        return o
    }

    /// Index into `skies` of the scene's sunset (0: none).
    var sky: Int {
        guard let ss = (parsed()["sky"] as? NSDictionary)?["sunset"] as? NSDictionary, (ss["enabled"] as? Bool) == true else { return 0 }
        return Look.skies.firstIndex(of: ss["style"] as? String ?? "Golden").map { max($0, 1) } ?? 1
    }

    /// Give the scene the sunset skies[i], the sun brought down low (and set at dusk); 0 takes it away.
    mutating func setSky(_ i: Int) {
        let s = parsed()
        let sky = Look.obj(s, "sky")
        let ss = Look.obj(sky, "sunset")
        ss["enabled"] = i > 0
        if i > 0 {
            ss["style"] = Look.skies[i]
            let sun = Look.obj(sky, "sun")
            var pos = (sun["pos"] as? [Double]) ?? [0.6, 0.9]
            if pos.count < 2 { pos = [0.6, 0.9] }
            if pos[1] < 0.8 { pos[1] = 0.88 }
            sun["pos"] = pos
            let dusk = i == 4
            sun["enabled"] = !dusk
            if !dusk {
                Look.obj(sky, "stars")["enabled"] = false
                // A night scene lit as an evening.
                let light = Look.obj(s, "light")
                light["ambient"] = max((light["ambient"] as? Double) ?? 0.6, 0.35)
            }
        }
        store(s)
    }

    /// Index into `times` of the scene's day and night (0: none).
    var time: Int {
        guard let dn = (parsed()["sky"] as? NSDictionary)?["day_night"] as? NSDictionary, (dn["enabled"] as? Bool) == true else { return 0 }
        if (dn["real_time"] as? Bool) == true { return 1 }
        if ((dn["days_per_loop"] as? Int) ?? 0) > 0 { return 6 }
        let h = (dn["hour"] as? Double) ?? 12
        return (2..<6).min(by: { abs(Look.hours[$0] - h) < abs(Look.hours[$1] - h) }) ?? 3
    }

    /// Set the time of day to times[i]: the clock, a fixed hour, or a day each loop; 0 as designed.
    mutating func setTime(_ i: Int) {
        let s = parsed()
        let dn = Look.obj(Look.obj(s, "sky"), "day_night")
        dn["enabled"] = i > 0
        dn["real_time"] = i == 1
        dn["days_per_loop"] = i == 6 ? 1 : 0
        if i >= 2 { dn["hour"] = Look.hours[i] }
        store(s)
    }

    var mixed: Bool { (((parsed()["sky"] as? NSDictionary)?["cloud_mix"] as? NSDictionary)?["enabled"] as? Bool) == true }

    /// A random mix of cloud kinds (`seed` deals another), or none.
    mutating func setMixed(_ on: Bool, seed: Int? = nil) {
        let s = parsed()
        let mix = Look.obj(Look.obj(s, "sky"), "cloud_mix")
        mix["enabled"] = on
        if on && mix["layers"] == nil { mix["layers"] = 3 }
        if on && ((mix["patches"] as? Double) ?? 0) <= 0 { mix["patches"] = 0.5 }
        if let seed = seed { mix["seed"] = seed }
        store(s)
    }

    /// Walking speed, metres a second.
    var speed: Double { ((parsed()["motion"] as? NSDictionary)?["speed"] as? Double) ?? 2.5 }

    mutating func setSpeed(_ v: Double) {
        let s = parsed()
        Look.obj(s, "motion")["speed"] = v
        store(s)
    }

    /// Whether the sky is drawn (dungeons and caves have none).
    var hasSky: Bool { ((parsed()["sky"] as? NSDictionary)?["enabled"] as? Bool) ?? true }

    /// Whether a scene draws in big pixels (then it is scaled up without smoothing).
    static func pixelated(_ scene: String) -> Bool {
        guard let data = scene.data(using: .utf8),
              let s = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
              let style = s["style"] as? [String: Any] else { return false }
        return ((style["pixel_size"] as? Int) ?? 1) > 1
    }

    /// `scene` with its name set to `name`.
    static func named(_ scene: String, _ name: String) -> String {
        guard let d = scene.data(using: .utf8),
              let o = (try? JSONSerialization.jsonObject(with: d, options: [.mutableContainers])) as? NSMutableDictionary else { return scene }
        o["name"] = name
        guard let out = try? JSONSerialization.data(withJSONObject: o), let s = String(data: out, encoding: .utf8) else { return scene }
        return s
    }
}

/// Your own scenes: one JSON file each, named after the scene, newest first.
enum Library {
    static var support: URL {
        let dir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private static var dir: URL {
        let d = support.appendingPathComponent("scenes")
        try? FileManager.default.createDirectory(at: d, withIntermediateDirectories: true)
        return d
    }

    static func file(_ name: String) -> URL {
        let safe = name.map { "/\\:*?\"<>|".contains($0) ? "_" : $0 }
        return dir.appendingPathComponent(String(safe) + ".json")
    }

    static func names() -> [String] {
        let fm = FileManager.default
        guard let urls = try? fm.contentsOfDirectory(at: dir, includingPropertiesForKeys: [.contentModificationDateKey]) else { return [] }
        let dated = urls.filter { $0.pathExtension == "json" }.map { url -> (String, Date) in
            let d = (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
            return (url.deletingPathExtension().lastPathComponent, d)
        }
        return dated.sorted { $0.1 > $1.1 }.map { $0.0 }
    }

    static func exists(_ name: String) -> Bool { FileManager.default.fileExists(atPath: file(name).path) }

    static func modified(_ name: String) -> Double {
        ((try? FileManager.default.attributesOfItem(atPath: file(name).path)[.modificationDate]) as? Date)?.timeIntervalSince1970 ?? 0
    }

    static func load(_ name: String) -> String? { try? String(contentsOf: file(name), encoding: .utf8) }

    @discardableResult
    static func save(_ name: String, _ scene: String) -> Bool { (try? scene.write(to: file(name), atomically: true, encoding: .utf8)) != nil }

    static func delete(_ name: String) { try? FileManager.default.removeItem(at: file(name)) }

    @discardableResult
    static func rename(_ from: String, _ to: String) -> Bool { (try? FileManager.default.moveItem(at: file(from), to: file(to))) != nil }
}
