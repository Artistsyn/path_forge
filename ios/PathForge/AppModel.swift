import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// A message shown in an alert.
struct Note: Identifiable {
    let id = UUID()
    let title: String
    let text: String
}

/// A name to ask for, and what to do with it.
struct Ask {
    let title: String
    let start: String
    let then: (String) -> Void
}

/// A question to confirm, and what to do when it is.
struct Confirm {
    let title: String
    let button: String
    let then: () -> Void
}

/// A scene as a file, for exporting it to Files (and from there to the desktop).
struct SceneDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.json] }
    var text: String
    init(text: String) { self.text = text }
    init(configuration: ReadConfiguration) throws {
        text = configuration.file.regularFileContents.flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }
    func fileWrapper(configuration: WriteConfiguration) throws -> FileWrapper { FileWrapper(regularFileWithContents: Data(text.utf8)) }
}

/// The app's state: the scene being worked on, yours and the presets, the editor, what is busy.
final class AppModel: ObservableObject {
    @Published var look: Look
    @Published var panelShown = true
    @Published var editing = false
    @Published var status = ""
    @Published var busy: String?
    @Published var progress: Double?
    @Published var note: Note?
    @Published var ask: Ask?
    @Published var askText = ""
    @Published var confirm: Confirm?
    @Published var importing = false
    @Published var exportDoc: SceneDocument?
    @Published var exportName = "Scene"
    @Published var thumbs: [String: UIImage] = [:]
    @Published var gpuOn: Bool
    @Published var mine: [String] = []
    let presets: [String]
    let editor: EditorModel
    weak var preview: PreviewUIView?
    private var gpuName = ""
    private var pending: DispatchWorkItem?
    let work = DispatchQueue(label: "pathforge.work", qos: .userInitiated)

    var scene: String { look.scene }

    init() {
        look = Look.load()
        presets = Native.presetNames()
        editor = EditorModel(form: Native.take(pfm_form()) ?? "{}")
        gpuOn = Gpu.wanted
        if Gpu.crashedLastTime() {
            gpuOn = false
            note = Note(title: "Drawing on the CPU", text: "The GPU stopped PathForge last time, so it draws on the CPU now. You can switch back under Draw on.")
        }
        if gpuOn { startGpu() }
        editor.onChange = { [weak self] json in self?.edited(json) }
        refreshLibrary()
        loadThumbs(presets.map { ("preset", $0) })
    }

    private func startGpu() {
        work.async { [weak self] in
            let name = Native.gpuInit()
            DispatchQueue.main.async { self?.gpuName = name }
        }
    }

    /// The scene changed: save it and show it.
    func changed() {
        look.save()
        preview?.setScene(scene)
    }

    /// A change from the editor: shown a moment later, so a slider being dragged is not redrawn
    /// for every step.
    private func edited(_ json: String) {
        look.scene = json
        pending?.cancel()
        let item = DispatchWorkItem { [weak self] in self?.changed() }
        pending = item
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: item)
    }

    func open(_ name: String, mine isMine: Bool) {
        guard let s = isMine ? Library.load(name) : Native.presetScene(name) else { return }
        look.name = name
        look.mine = isMine
        look.scene = s
        changed()
    }

    // ── Quick controls ───────────────────────────────────────────────────

    func setSky(_ i: Int) {
        guard look.hasSky else { note = Note(title: "This scene has no sky", text: "Switch it on under Edit every setting → Sky."); return }
        look.setSky(i)
        changed()
    }

    func setMix(_ on: Bool) { look.setMixed(on); changed() }

    func shuffle() { look.setMixed(true, seed: Int.random(in: 0..<9999)); changed() }

    func setSpeed(_ v: Double) { look.setSpeed((v * 10).rounded() / 10) }

    func setGpu(_ on: Bool) {
        Gpu.wanted = on
        gpuOn = on
        if on && gpuName.isEmpty { startGpu() }
        preview?.reopen(scene)
    }

    func showStats(gpu: Bool, width: Int, height: Int, fps: Double) {
        let engine = gpu ? (gpuName.isEmpty ? "GPU" : "GPU \(gpuName)") : "CPU"
        status = String(format: "%@  ·  %d×%d  ·  %.0f fps", engine, width, height, fps)
    }

    // ── The editor ───────────────────────────────────────────────────────

    func openEditor() {
        editor.load(scene)
        editing = true
        preview?.setFit(true)
    }

    func closeEditor() {
        pending?.cancel()
        changed()
        editing = false
        preview?.setFit(false)
        if look.mine { note = Note(title: "Changed", text: "Save scene keeps the changes in \(look.name).") }
    }

    // ── Your scenes ──────────────────────────────────────────────────────

    func refreshLibrary() {
        mine = Library.names()
        loadThumbs(mine.map { ("mine", $0) })
    }

    func askName(_ title: String, _ start: String, then: @escaping (String) -> Void) {
        askText = start
        ask = Ask(title: title, start: start, then: then)
    }

    /// Keep `scene` (the current one when nil) as one of yours, asking for its name.
    func save(suggest: String, scene given: String? = nil) {
        askName("Save scene as", suggest) { n in
            let doSave = {
                let s = Look.named(given ?? self.look.scene, n)
                guard Library.save(n, s) else { self.note = Note(title: "Could not save it", text: n); return }
                self.look.name = n
                self.look.mine = true
                self.look.scene = s
                self.changed()
                self.refreshLibrary()
            }
            let same = self.look.mine && self.look.name == n && given == nil
            if Library.exists(n) && !same { self.confirm = Confirm(title: "Replace your scene \(n)?", button: "Replace", then: doSave) } else { doSave() }
        }
    }

    func rename(_ name: String) {
        askName("Rename", name) { to in
            guard !Library.exists(to) else { self.note = Note(title: "There is already a scene called \(to)", text: ""); return }
            guard Library.rename(name, to) else { return }
            if let s = Library.load(to) { Library.save(to, Look.named(s, to)) }
            if self.look.mine && self.look.name == name { self.look.name = to; self.look.scene = Library.load(to) ?? self.look.scene; self.changed() }
            self.refreshLibrary()
        }
    }

    func delete(_ name: String) {
        confirm = Confirm(title: "Delete \(name)?", button: "Delete") {
            Library.delete(name)
            if self.look.mine && self.look.name == name { self.look.mine = false; self.look.save() }
            self.refreshLibrary()
        }
    }

    func export(_ name: String) {
        guard let s = Library.load(name) else { return }
        exportName = name
        exportDoc = SceneDocument(text: s)
    }

    /// A small first frame of each scene, made once and kept (presets per version of the app,
    /// yours per save).
    private func loadThumbs(_ which: [(String, String)]) {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "0"
        let dir = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0].appendingPathComponent("thumbs")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        DispatchQueue.global(qos: .utility).async { [weak self] in
            for (kind, name) in which {
                let stamp = kind == "mine" ? String(Int(Library.modified(name))) : version
                let file = dir.appendingPathComponent("\(kind)-\(abs(name.hashValue))-\(stamp).png")
                var img = UIImage(contentsOfFile: file.path)
                if img == nil, let json = kind == "mine" ? Library.load(name) : Native.presetScene(name),
                   // Frames this small are quicker on the CPU than a trip to the GPU and back.
                   let s = PathScene(json: json, gpu: false),
                   let cg = s.image(distance: 0, time: 0, width: 90, height: 160) {
                    img = UIImage(cgImage: cg)
                    try? img?.pngData()?.write(to: file)
                }
                if let img = img { DispatchQueue.main.async { self?.thumbs["\(kind):\(name)"] = img } }
            }
        }
    }

    /// A scene file made on the desktop.
    func importScene(_ result: Result<URL, Error>) {
        guard case .success(let url) = result else { return }
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        guard let data = try? Data(contentsOf: url), let json = String(data: data, encoding: .utf8) else {
            note = Note(title: "Could not read the file", text: url.lastPathComponent)
            return
        }
        guard let scene = Native.normalize(json) else {
            note = Note(title: "Not a PathForge scene", text: Native.lastError)
            return
        }
        var name = "Imported scene"
        if let d = scene.data(using: .utf8), let o = (try? JSONSerialization.jsonObject(with: d)) as? [String: Any],
           let n = o["name"] as? String, !n.isEmpty { name = n }
        look.name = name
        look.mine = false
        look.scene = scene
        changed()
        note = Note(title: "Imported \(name)", text: "Save scene keeps it with yours. Scenes that use image files (kits, sprites) show without them.")
    }
}
