import SwiftUI
import UIKit

/// One step of a path into the scene's JSON: a key, or a list index.
enum PathKey: Hashable {
    case key(String)
    case index(Int)
}

/// Every setting of the scene, from the renderer's description of them (pfm_form: the same schema
/// and slider ranges as the desktop studio's inspector), and the scene being edited.
final class EditorModel: ObservableObject {
    let parts: [[String: Any]]
    @Published var part = 0
    @Published var advanced = false
    @Published var open: Set<String> = []
    @Published var help: Note?
    @Published private(set) var canUndo = false
    @Published private(set) var canRedo = false
    private(set) var root = NSMutableDictionary()
    private var undo: [Data] = []
    private var redo: [Data] = []
    /// The scene as it was when editing began, for Before.
    private(set) var original = "{}"
    /// Told the scene, as JSON, after each change.
    var onChange: ((String) -> Void)?
    /// Told to show a scene for a moment without changing anything (nil: the edited one again).
    var onPeek: ((String?) -> Void)?

    init(form: String) {
        let o = form.data(using: .utf8).flatMap { try? JSONSerialization.jsonObject(with: $0) } as? [String: Any]
        parts = (o?["parts"] as? [[String: Any]]) ?? []
    }

    func load(_ json: String) {
        root = (json.data(using: .utf8).flatMap { try? JSONSerialization.jsonObject(with: $0, options: [.mutableContainers]) } as? NSMutableDictionary) ?? NSMutableDictionary()
        undo = []
        redo = []
        canUndo = false
        canRedo = false
        original = json
        objectWillChange.send()
    }

    var json: String {
        guard let d = try? JSONSerialization.data(withJSONObject: root), let s = String(data: d, encoding: .utf8) else { return "{}" }
        return s
    }

    func get(_ path: [PathKey]) -> Any? {
        var at: Any? = root
        for k in path {
            switch k {
            case .key(let s): at = (at as? NSDictionary)?[s]
            case .index(let i):
                guard let a = at as? NSArray, i < a.count else { return nil }
                at = a[i]
            }
            if at == nil || at is NSNull { return nil }
        }
        return at
    }

    private func put(_ path: [PathKey], _ value: Any) {
        var at: Any = root
        for k in path.dropLast() {
            switch k {
            case .key(let s):
                guard let d = at as? NSMutableDictionary else { return }
                if !(d[s] is NSMutableDictionary) && !(d[s] is NSMutableArray) { d[s] = NSMutableDictionary() }
                at = d[s]!
            case .index(let i):
                guard let a = at as? NSMutableArray, i < a.count else { return }
                at = a[i]
            }
        }
        switch path.last {
        case .key(let s)?: (at as? NSMutableDictionary)?[s] = value
        case .index(let i)?: if let a = at as? NSMutableArray, i < a.count { a[i] = value }
        case nil: break
        }
    }

    /// A change: remember the scene as it was (for Undo) when `step`, make it, pass it on.
    func set(_ path: [PathKey], _ value: Any, step: Bool) {
        if step { remember() }
        // Lists and objects are stored as mutable copies, so what is inside them can be edited later.
        let isTree = value is NSArray || value is NSDictionary || value is [Any] || value is [String: Any]
        put(path, isTree ? EditorModel.copy(value) : value)
        objectWillChange.send()
        onChange?(json)
    }

    func remember() {
        if let d = try? JSONSerialization.data(withJSONObject: root) { undo.append(d) }
        if undo.count > 100 { undo.removeFirst() }
        redo = []
        canUndo = true
        canRedo = false
    }

    func stepBack() { step(&undo, &redo) }
    func stepForward() { step(&redo, &undo) }

    /// Go back (or forward again): the scene from `from`, keeping this one in `to`.
    private func step(_ from: inout [Data], _ to: inout [Data]) {
        guard let d = from.popLast() else { return }
        if let now = try? JSONSerialization.data(withJSONObject: root) { to.append(now) }
        if to.count > 100 { to.removeFirst() }
        root = ((try? JSONSerialization.jsonObject(with: d, options: [.mutableContainers])) as? NSMutableDictionary) ?? root
        canUndo = !undo.isEmpty
        canRedo = !redo.isEmpty
        objectWillChange.send()
        onChange?(json)
    }

    static func key(_ path: [PathKey]) -> String {
        path.map { k -> String in
            switch k { case .key(let s): return s; case .index(let i): return String(i) }
        }.joined(separator: ".")
    }

    func isOpen(_ path: [PathKey]) -> Bool { open.contains(EditorModel.key(path)) }

    func toggle(_ path: [PathKey]) {
        let k = EditorModel.key(path)
        if open.contains(k) { open.remove(k) } else { open.insert(k) }
    }

    /// A deep copy of a JSON value (for list items).
    static func copy(_ v: Any) -> Any {
        guard JSONSerialization.isValidJSONObject(v), let d = try? JSONSerialization.data(withJSONObject: v),
              let c = try? JSONSerialization.jsonObject(with: d, options: [.mutableContainers]) else { return v }
        return c
    }
}

/// A field's description from the form.
struct Field {
    let d: [String: Any]
    var key: String { d["key"] as? String ?? "" }
    var label: String { d["label"] as? String ?? key }
    var kind: String { d["kind"] as? String ?? "" }
    var help: String { (d["help"] as? String ?? "").replacingOccurrences(of: "`", with: "") }
    var adv: Bool { d["adv"] as? Bool ?? false }
    var fields: [Field] { (d["fields"] as? [[String: Any]] ?? []).map(Field.init) }
    var choices: [[String: Any]] { d["choices"] as? [[String: Any]] ?? [] }
    var def: Any? { d["default"] }
    var min: Double? { (d["min"] as? NSNumber)?.doubleValue }
    var max: Double? { (d["max"] as? NSNumber)?.doubleValue }
    var unit: String { d["unit"] as? String ?? "" }
    var isInt: Bool { d["int"] as? Bool ?? false }
    var item: Field? { (d["item"] as? [String: Any]).map(Field.init) }
    var one: String { d["one"] as? String ?? "item" }
    var hasEnabled: Bool { fields.contains { $0.key == "enabled" } }

    func choiceLabel(_ value: String) -> String {
        (choices.first { $0["value"] as? String == value }?["label"] as? String) ?? value
    }
}

let accent = Color(red: 1, green: 0.61, blue: 0.33)

struct EditorView: View {
    @ObservedObject var ed: EditorModel
    let done: () -> Void
    @State private var peeking = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Edit").font(.title3.bold())
                Spacer()
                Button("Undo") { ed.stepBack() }.buttonStyle(Pill()).disabled(!ed.canUndo).opacity(ed.canUndo ? 1 : 0.4)
                Button("Redo") { ed.stepForward() }.buttonStyle(Pill()).disabled(!ed.canRedo).opacity(ed.canRedo ? 1 : 0.4)
                // Hold to see the scene as it was before these edits.
                Text("Before").font(.subheadline.weight(.semibold)).padding(.horizontal, 14).padding(.vertical, 8)
                    .background(Capsule().fill(Color.white.opacity(peeking ? 0.3 : 0.12)))
                    .gesture(DragGesture(minimumDistance: 0)
                        .onChanged { _ in if !peeking { peeking = true; ed.onPeek?(ed.original) } }
                        .onEnded { _ in peeking = false; ed.onPeek?(nil) })
                Button("Done", action: done).buttonStyle(Pill(primary: true))
            }
            Text("Long-press a setting's name for what it does. Hold Before to compare.").font(.caption).foregroundColor(.secondary)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(Array(visibleParts.enumerated()), id: \.offset) { _, item in
                        Button(item.1.label) { ed.part = item.0 }.buttonStyle(Pill(primary: item.0 == ed.part))
                    }
                    Button(ed.advanced ? "Fewer settings" : "All settings") { ed.advanced.toggle() }.buttonStyle(Pill())
                }
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 4) {
                    if ed.part < ed.parts.count {
                        let p = Field(d: ed.parts[ed.part])
                        if p.kind == "list" { ListView(ed: ed, f: p, path: [.key(p.key)], depth: 0) }
                        else { FieldsView(ed: ed, group: p, path: [.key(p.key)], depth: 0) }
                    }
                }
                .padding(.bottom, 24)
            }
        }
        .alert(item: $ed.help) { n in Alert(title: Text(n.title), message: Text(n.text), dismissButton: .default(Text("OK"))) }
    }

    private var visibleParts: [(Int, Field)] {
        ed.parts.enumerated().map { ($0.offset, Field(d: $0.element)) }.filter { ed.advanced || !$0.1.adv }
    }
}

/// A struct's settings, one after another (its `enabled` switch left out when `skipEnabled`).
struct FieldsView: View {
    @ObservedObject var ed: EditorModel
    let group: Field
    let path: [PathKey]
    let depth: Int
    var skipEnabled = false

    var body: some View {
        ForEach(Array(group.fields.enumerated()), id: \.offset) { _, f in
            if (ed.advanced || !f.adv) && !(skipEnabled && f.key == "enabled") {
                FieldView(ed: ed, f: f, path: path + [.key(f.key)], depth: depth)
            }
        }
    }
}

struct FieldView: View {
    @ObservedObject var ed: EditorModel
    let f: Field
    let path: [PathKey]
    let depth: Int

    var body: some View {
        switch f.kind {
        case "bool": boolView
        case "number": numberView
        case "enum": enumView
        case "set": setView
        case "color": colorView
        case "colors": colorsView
        case "pair": pairView
        case "group": AnyView(GroupView(ed: ed, f: f, path: path, depth: depth))
        case "list": AnyView(ListView(ed: ed, f: f, path: path, depth: depth))
        default: AnyView(EmptyView())
        }
    }

    private var name: some View {
        Text(f.label).onLongPressGesture { if !f.help.isEmpty { ed.help = Note(title: f.label, text: f.help) } }
    }

    private var boolView: AnyView {
        AnyView(Toggle(isOn: Binding(get: { (ed.get(path) as? Bool) ?? (f.def as? Bool) ?? false },
                                     set: { ed.set(path, $0, step: true) })) { name }
            .tint(accent).frame(minHeight: 44))
    }

    private func number() -> Double { (ed.get(path) as? NSNumber)?.doubleValue ?? (f.def as? NSNumber)?.doubleValue ?? 0 }

    private func fmt(_ v: Double) -> String {
        let s = f.isInt ? String(Int(v.rounded())) : String(format: "%.3f", v).replacingOccurrences(of: "\\.?0+$", with: "", options: .regularExpression)
        return f.unit.isEmpty ? s : "\(s) \(f.unit)"
    }

    private func value(_ x: Double) -> Any { f.isInt ? NSNumber(value: Int(x.rounded())) : NSNumber(value: x) }

    private var numberView: AnyView {
        if let lo = f.min, let hi = f.max, hi > lo {
            return AnyView(VStack(alignment: .leading, spacing: 0) {
                HStack { name; Spacer(); Text(fmt(number())).foregroundColor(accent).monospacedDigit() }
                Slider(value: Binding(get: { Swift.min(Swift.max(number(), lo), hi) }, set: { ed.set(path, value($0), step: false) }),
                       in: lo...hi, step: f.isInt ? 1 : (hi - lo) / 1000,
                       onEditingChanged: { if $0 { ed.remember() } }).tint(accent)
            }.padding(.vertical, 4))
        }
        let floor = f.min ?? -1e9
        return AnyView(HStack {
            name
            Spacer()
            Text(fmt(number())).foregroundColor(accent).monospacedDigit()
            Stepper("", onIncrement: { step(1, floor) }, onDecrement: { step(-1, floor) }).labelsHidden()
        }.frame(minHeight: 44))
    }

    private func step(_ sign: Double, _ floor: Double) {
        let cur = number()
        let by = f.isInt ? 1 : (abs(cur) >= 10 ? 1 : 0.1)
        ed.set(path, value(Swift.max(floor, ((cur + sign * by) * 1000).rounded() / 1000)), step: true)
    }

    private var enumView: AnyView {
        let cur = (ed.get(path) as? String) ?? (f.def as? String) ?? ""
        return AnyView(HStack {
            name
            Spacer()
            Menu {
                ForEach(Array(f.choices.enumerated()), id: \.offset) { _, c in
                    let v = c["value"] as? String ?? ""
                    Button { ed.set(path, v, step: true) } label: {
                        if v == cur { Label(c["label"] as? String ?? v, systemImage: "checkmark") } else { Text(c["label"] as? String ?? v) }
                        if let h = c["help"] as? String, !h.isEmpty { Text(h.replacingOccurrences(of: "`", with: "")) }
                    }
                }
            } label: {
                Text(f.choiceLabel(cur) + " ▾").padding(.horizontal, 12).padding(.vertical, 8)
                    .background(Capsule().fill(Color.white.opacity(0.1)))
            }
        }.frame(minHeight: 44))
    }

    private var setView: AnyView {
        let have = (ed.get(path) as? [String]) ?? []
        return AnyView(HStack {
            name
            Spacer()
            Menu {
                ForEach(Array(f.choices.enumerated()), id: \.offset) { _, c in
                    let v = c["value"] as? String ?? ""
                    Button {
                        var next = have
                        if let i = next.firstIndex(of: v) { next.remove(at: i) } else { next.append(v) }
                        ed.set(path, next, step: true)
                    } label: {
                        if have.contains(v) { Label(c["label"] as? String ?? v, systemImage: "checkmark") } else { Text(c["label"] as? String ?? v) }
                    }
                }
            } label: {
                Text((have.isEmpty ? "Any" : have.map(f.choiceLabel).joined(separator: ", ")) + " ▾").lineLimit(1)
                    .padding(.horizontal, 12).padding(.vertical, 8).background(Capsule().fill(Color.white.opacity(0.1)))
            }
        }.frame(minHeight: 44))
    }

    private static func color(_ v: Any?) -> Color {
        guard let a = v as? [NSNumber], a.count >= 3 else { return .gray }
        return Color(.sRGB, red: a[0].doubleValue / 255, green: a[1].doubleValue / 255, blue: a[2].doubleValue / 255, opacity: 1)
    }

    private static func rgb(_ c: Color) -> [Int] {
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        UIColor(c).getRed(&r, green: &g, blue: &b, alpha: &a)
        let k = { (x: CGFloat) in Int((Swift.min(Swift.max(x, 0), 1) * 255).rounded()) }
        return [k(r), k(g), k(b)]
    }

    private var colorView: AnyView {
        AnyView(ColorPicker(selection: Binding(get: { FieldView.color(ed.get(path) ?? f.def) },
                                               set: { ed.set(path, FieldView.rgb($0), step: true) }), supportsOpacity: false) { name }
            .frame(minHeight: 44))
    }

    private var colorsView: AnyView {
        let n = f.d["count"] as? Int ?? 2
        return AnyView(HStack {
            name
            Spacer()
            ForEach(0..<n, id: \.self) { i in
                ColorPicker("", selection: Binding(get: {
                    FieldView.color((ed.get(path) as? [Any])?[safe: i] ?? (f.def as? [Any])?[safe: i])
                }, set: { c in
                    var all = ((ed.get(path) ?? f.def) as? [Any]) ?? []
                    while all.count <= i { all.append([128, 128, 128]) }
                    all[i] = FieldView.rgb(c)
                    ed.set(path, all, step: true)
                }), supportsOpacity: false).labelsHidden()
            }
        }.frame(minHeight: 44))
    }

    private var pairView: AnyView {
        AnyView(VStack(alignment: .leading, spacing: 0) {
            name.padding(.top, 6)
            ForEach(0..<2, id: \.self) { k in
                HStack {
                    Text(k == 0 ? "Across" : "Down").foregroundColor(.secondary).frame(width: 64, alignment: .leading)
                    Slider(value: Binding(get: {
                        ((ed.get(path) ?? f.def) as? [NSNumber])?[safe: k]?.doubleValue ?? 0.5
                    }, set: { x in
                        var xy = ((ed.get(path) ?? f.def) as? [NSNumber])?.map { $0.doubleValue } ?? [0.5, 0.5]
                        while xy.count < 2 { xy.append(0.5) }
                        xy[k] = x
                        ed.set(path, xy, step: false)
                    }), in: 0...1, onEditingChanged: { if $0 { ed.remember() } }).tint(accent)
                }
            }
        })
    }
}

extension Array {
    subscript(safe i: Int) -> Element? { i >= 0 && i < count ? self[i] : nil }
}

/// A struct's settings under a header that opens and closes it; its `enabled` switch, if it has
/// one, sits in the header.
struct GroupView: View {
    @ObservedObject var ed: EditorModel
    let f: Field
    let path: [PathKey]
    let depth: Int
    var title: String? = nil
    var extra: AnyView? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text((ed.isOpen(path) ? "▾  " : "▸  ") + (title ?? f.label)).bold()
                    .onLongPressGesture { if !f.help.isEmpty { ed.help = Note(title: title ?? f.label, text: f.help) } }
                Spacer()
                if let extra = extra { extra }
                if f.hasEnabled {
                    Toggle("", isOn: Binding(get: { (ed.get(path + [.key("enabled")]) as? Bool) ?? false },
                                             set: { on in
                                                 ed.set(path + [.key("enabled")], on, step: true)
                                                 if on && !ed.isOpen(path) { ed.toggle(path) }
                                             })).labelsHidden().tint(accent)
                }
            }
            .frame(minHeight: 44)
            .contentShape(Rectangle())
            .onTapGesture { withAnimation(.easeOut(duration: 0.15)) { ed.toggle(path) } }
            if ed.isOpen(path) {
                VStack(alignment: .leading, spacing: 4) {
                    FieldsView(ed: ed, group: f, path: path, depth: depth + 1, skipEnabled: true)
                }
                .padding(.leading, 12).padding(.vertical, 4)
                .background(RoundedRectangle(cornerRadius: 12).fill(Color.white.opacity(depth % 2 == 0 ? 0.07 : 0.04)))
            }
        }
    }
}

/// A list: its items, each opening to its settings, with copy and remove; and Add.
struct ListView: View {
    @ObservedObject var ed: EditorModel
    let f: Field
    let path: [PathKey]
    let depth: Int

    var body: some View {
        let items = (ed.get(path) as? [Any]) ?? []
        let one = f.one
        let One = one.prefix(1).uppercased() + one.dropFirst()
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text("\(f.label)  (\(items.count))").font(.headline)
                Spacer()
                Button("+ Add \(one)") {
                    var next = items
                    next.append(EditorModel.copy(f.d["new"] ?? [String: Any]()))
                    ed.set(path, next, step: true)
                    ed.open.insert(EditorModel.key(path + [.index(next.count - 1)]))
                }.buttonStyle(Pill())
            }.frame(minHeight: 44)
            if let item = f.item {
                ForEach(0..<items.count, id: \.self) { i in
                    let kind = (items[i] as? [String: Any])?["kind"] as? String ?? ""
                    let kindField = item.fields.first { $0.key == "kind" }
                    let label = "\(One) \(i + 1)" + (kind.isEmpty ? "" : " · " + (kindField?.choiceLabel(kind) ?? kind))
                    GroupView(ed: ed, f: item, path: path + [.index(i)], depth: depth + 1, title: label, extra: AnyView(Menu {
                        Button("Copy") {
                            var next = items
                            next.insert(EditorModel.copy(items[i]), at: i + 1)
                            ed.set(path, next, step: true)
                        }
                        Button("Remove", role: .destructive) {
                            var next = items
                            next.remove(at: i)
                            ed.open.remove(EditorModel.key(path + [.index(i)]))
                            ed.set(path, next, step: true)
                        }
                    } label: { Image(systemName: "ellipsis.circle").font(.title3) }))
                }
            }
        }
    }
}
