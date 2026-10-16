import SwiftUI
import CoreImage
import UIKit
import UniformTypeIdentifiers

/// A rounded button: filled with the accent colour when `primary`, outlined otherwise.
struct Pill: ButtonStyle {
    var primary = false
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.subheadline.bold())
            .lineLimit(1)
            .padding(.horizontal, 14).padding(.vertical, 10)
            .frame(minHeight: 40)
            .foregroundColor(primary ? Color(red: 0.12, green: 0.07, blue: 0.04) : .white)
            .background(Capsule().fill(primary ? accent : Color.white.opacity(0.08)))
            .overlay(Capsule().stroke(Color.white.opacity(primary ? 0 : 0.35), lineWidth: 1))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

/// A scrolling row of choices, one selected.
struct Chips: View {
    let labels: [String]
    let selected: Int
    let pick: (Int) -> Void
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(labels.indices, id: \.self) { i in
                    Button(labels[i]) { pick(i) }.buttonStyle(Pill(primary: i == selected))
                }
            }
        }
    }
}

struct SectionLabel: View {
    let text: String
    var body: some View { Text(text.uppercased()).font(.caption.weight(.semibold)).tracking(1).foregroundColor(.secondary).padding(.top, 8) }
}

struct ContentView: View {
    @StateObject private var model = AppModel()

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .top) {
                Color.black.ignoresSafeArea()
                PreviewView(model: model)
                    .frame(maxWidth: .infinity, maxHeight: model.editing ? geo.size.height * 0.4 : .infinity, alignment: .top)
                    .ignoresSafeArea(edges: model.editing ? [] : .all)
                    .onTapGesture { if !model.editing { withAnimation(.easeOut(duration: 0.2)) { model.panelShown.toggle() } } }
                VStack {
                    Spacer()
                    if model.editing {
                        EditorView(ed: model.editor) { model.closeEditor() }
                            .padding(16)
                            .frame(height: geo.size.height * 0.62)
                            .background(Sheet().ignoresSafeArea(edges: .bottom))
                    } else if model.panelShown {
                        Panel(model: model)
                            .transition(.move(edge: .bottom).combined(with: .opacity))
                    } else {
                        HStack {
                            Spacer()
                            Button("Edit") { withAnimation { model.panelShown = true } }.buttonStyle(Pill()).padding()
                        }
                    }
                }
                if let busy = model.busy {
                    ZStack {
                        Color.black.opacity(0.55).ignoresSafeArea()
                        VStack(spacing: 14) {
                            Text(busy).font(.headline)
                            if let p = model.progress { ProgressView(value: p).tint(accent).frame(width: 220) } else { ProgressView() }
                            if model.cancellable { Button("Cancel") { pfm_export_cancel() }.buttonStyle(Pill()) }
                        }
                        .padding(24)
                        .background(RoundedRectangle(cornerRadius: 20).fill(Color(white: 0.12)))
                    }
                }
            }
        }
        .background(Color.clear.sheet(isPresented: $model.askingAnimation) { AnimationSheet(model: model) })
        .background(Color.clear.sheet(isPresented: $model.askingLocks) { LocksSheet(model: model) })
        .background(Color.clear.sheet(item: $model.sharing) { c in CodeSheet(code: c) })
        .background(Color.clear.alert("Open a share code", isPresented: $model.openingCode) {
            TextField("PF1.…", text: $model.codeText)
            Button("Open") { model.open(code: model.codeText) }
            Button("Cancel", role: .cancel) { }
        })
        .background(Color.clear.sheet(item: Binding(get: { model.shareURL.map { SharedFile(url: $0) } }, set: { if $0 == nil { model.shareURL = nil } })) { f in
            ShareSheet(items: [f.url])
        })
        .preferredColorScheme(.dark)
        .statusBarHidden(!model.panelShown && !model.editing)
        .fileImporter(isPresented: $model.importing, allowedContentTypes: [.json, .plainText, .data], allowsMultipleSelection: false) { result in
            switch result {
            case .success(let urls): if let u = urls.first { model.importScene(.success(u)) }
            case .failure(let e): model.importScene(.failure(e))
            }
        }
        .fileExporter(isPresented: Binding(get: { model.exportDoc != nil }, set: { if !$0 { model.exportDoc = nil } }),
                      document: model.exportDoc, contentType: .json, defaultFilename: model.exportName) { _ in model.exportDoc = nil }
        .background(Color.clear.alert(item: $model.note) { n in Alert(title: Text(n.title), message: Text(n.text), dismissButton: .default(Text("OK"))) })
        .background(Color.clear.alert(model.ask?.title ?? "", isPresented: Binding(get: { model.ask != nil }, set: { if !$0 { model.ask = nil } })) {
            TextField("Name", text: $model.askText)
            Button("OK") {
                let n = model.askText.trimmingCharacters(in: .whitespaces)
                let then = model.ask?.then
                model.ask = nil
                if !n.isEmpty { DispatchQueue.main.async { then?(n) } }
            }
            Button("Cancel", role: .cancel) { model.ask = nil }
        })
        .background(Color.clear.alert(model.confirm?.title ?? "", isPresented: Binding(get: { model.confirm != nil }, set: { if !$0 { model.confirm = nil } })) {
            Button(model.confirm?.button ?? "OK", role: .destructive) {
                let then = model.confirm?.then
                model.confirm = nil
                then?()
            }
            Button("Cancel", role: .cancel) { model.confirm = nil }
        })
    }
}

/// The panel's background: dark, rounded at the top.
struct Sheet: View {
    var body: some View {
        UnevenRounded().fill(Color(red: 0.08, green: 0.07, blue: 0.12).opacity(0.94))
    }
}

/// A rectangle rounded at its top corners only.
struct UnevenRounded: Shape {
    func path(in r: CGRect) -> Path {
        let k: CGFloat = 24
        var p = Path()
        p.move(to: CGPoint(x: r.minX, y: r.maxY))
        p.addLine(to: CGPoint(x: r.minX, y: r.minY + k))
        p.addQuadCurve(to: CGPoint(x: r.minX + k, y: r.minY), control: CGPoint(x: r.minX, y: r.minY))
        p.addLine(to: CGPoint(x: r.maxX - k, y: r.minY))
        p.addQuadCurve(to: CGPoint(x: r.maxX, y: r.minY + k), control: CGPoint(x: r.maxX, y: r.minY))
        p.addLine(to: CGPoint(x: r.maxX, y: r.maxY))
        p.closeSubpath()
        return p
    }
}

struct Panel: View {
    @ObservedObject var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text((model.look.mine ? "★ " : "") + model.look.name).font(.title3.bold()).lineLimit(1)
                    Text(model.status).font(.caption).foregroundColor(.secondary).lineLimit(1)
                }
                Spacer()
                Button("Import") { model.importing = true }.buttonStyle(Pill())
                Button("Hide") { withAnimation { model.panelShown = false } }.buttonStyle(Pill())
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    SectionLabel(text: "Scenes · long-press for more")
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(alignment: .top, spacing: 10) {
                            ForEach(model.mine, id: \.self) { n in tile(n, mine: true) }
                            if !model.mine.isEmpty { Rectangle().fill(Color.white.opacity(0.25)).frame(width: 1, height: 120) }
                            ForEach(model.presets, id: \.self) { n in tile(n, mine: false) }
                        }
                    }
                    SectionLabel(text: "Sunset")
                    Chips(labels: Look.skies, selected: model.look.sky) { model.setSky($0) }
                    SectionLabel(text: "Time of day")
                    Chips(labels: Look.times, selected: model.look.time) { model.setTime($0) }
                    SectionLabel(text: "Season")
                    Chips(labels: Look.seasons, selected: model.look.season) { model.setSeason($0) }
                    SectionLabel(text: "Clouds")
                    HStack {
                        Chips(labels: ["As designed", "Mixed"], selected: model.look.mixed ? 1 : 0) { model.setMix($0 == 1) }
                        Button("Shuffle") { model.shuffle() }.buttonStyle(Pill())
                    }
                    SectionLabel(text: String(format: "Walking speed  %.1f m/s", model.look.speed))
                    Slider(value: Binding(get: { model.look.speed }, set: { model.setSpeed($0) }), in: 0.5...12,
                           onEditingChanged: { if !$0 { model.changed() } }).tint(accent)
                    SectionLabel(text: "Draw on")
                    Chips(labels: ["GPU (fast)", "CPU"], selected: model.gpuOn ? 0 : 1) { model.setGpu($0 == 0) }
                    SectionLabel(text: "Preview")
                    Chips(labels: ["Auto (smooth)", "Sharp (full size)"], selected: model.sharp ? 1 : 0) { model.setSharp($0 == 1) }
                    SectionLabel(text: "Wallpaper")
                    Text("iPhone apps can't set the wallpaper themselves. Save it to Photos below, then in Photos tap Share → Use as Wallpaper. A Live Photo moves on the lock screen (iOS 17+).")
                        .font(.caption).foregroundColor(.secondary)
                }
            }
            .frame(maxHeight: UIScreen.main.bounds.height * 0.34)
            HStack {
                Button("Edit every setting") { model.openEditor() }.buttonStyle(Pill())
                Button("Save scene") { model.save(suggest: model.look.name) }.buttonStyle(Pill())
            }
            HStack {
                Button("Surprise me") { model.surprise() }.buttonStyle(Pill())
                    .simultaneousGesture(LongPressGesture().onEnded { _ in model.askingLocks = true })
                Button("Locks") { model.askingLocks = true }.buttonStyle(Pill())
                Button("Back") { model.back() }.buttonStyle(Pill()).disabled(!model.canGoBack).opacity(model.canGoBack ? 1 : 0.4)
            }
            HStack {
                Button("Share code / QR") { model.shareCode() }.buttonStyle(Pill())
                Button("Open a code") { model.openCode() }.buttonStyle(Pill())
            }
            HStack {
                Button("Still") { model.saveStill() }.buttonStyle(Pill(primary: true))
                Button("Live Photo") { model.saveLivePhoto() }.buttonStyle(Pill(primary: true))
                Button("Video") { model.saveVideo() }.buttonStyle(Pill())
                Button("GIF/WebP") { model.askingAnimation = true }.buttonStyle(Pill())
            }
        }
        .padding(16)
        .background(Sheet().ignoresSafeArea(edges: .bottom))
    }

    private func tile(_ name: String, mine: Bool) -> some View {
        let on = model.look.name == name && model.look.mine == mine
        return VStack(spacing: 4) {
            ZStack {
                RoundedRectangle(cornerRadius: 10).fill(on ? accent : Color.white.opacity(0.12))
                if let img = model.thumbs["\(mine ? "mine" : "preset"):\(name)"] {
                    Image(uiImage: img).resizable().scaledToFill().frame(width: 64, height: 114).clipShape(RoundedRectangle(cornerRadius: 8))
                }
            }
            .frame(width: 68, height: 120)
            Text(mine ? "★ \(name)" : name).font(.caption2).multilineTextAlignment(.center).lineLimit(2).frame(width: 68)
        }
        .onTapGesture { model.open(name, mine: mine) }
        .contextMenu {
            if mine {
                Button("Rename") { model.rename(name) }
                Button("Duplicate") { if let s = Library.load(name) { model.save(suggest: name + " copy", scene: s) } }
                Button("Export to Files") { model.export(name) }
                Button("Delete", role: .destructive) { model.delete(name) }
            } else {
                Button("Save a copy as one of yours") { if let s = Native.presetScene(name) { model.save(suggest: name + " (mine)", scene: s) } }
            }
        }
    }
}


/// A file to share, as an item for `.sheet(item:)`.
struct SharedFile: Identifiable {
    let url: URL
    var id: String { url.path }
}

/// The system share sheet (Save Image, Save to Files, Messages, AirDrop...).
struct ShareSheet: UIViewControllerRepresentable {
    let items: [Any]
    func makeUIViewController(context: Context) -> UIActivityViewController { UIActivityViewController(activityItems: items, applicationActivities: nil) }
    func updateUIViewController(_ vc: UIActivityViewController, context: Context) {}
}

/// Choosing how to save the loop as an animation.
struct AnimationSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss
    @AppStorage("anim.format") private var format = 0
    @AppStorage("anim.size") private var size = 1
    @AppStorage("anim.fps") private var fps = 1

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Save the loop as an animation").font(.title3.bold())
            SectionLabel(text: "Format")
            Chips(labels: ["WebP (full colour, small)", "GIF (plays anywhere)"], selected: format) { format = $0 }
            SectionLabel(text: "Size")
            Chips(labels: ["Small", "Medium", "Large", "Full screen"], selected: size) { size = $0 }
            SectionLabel(text: "Frames a second")
            Chips(labels: ["12", "20", "30"], selected: fps) { fps = $0 }
            Text("The whole loop, joining end to start, so it plays forever without a seam. Larger and smoother takes longer and makes a bigger file (GIF most of all). Photos plays GIFs; WebP is smaller and shows every colour.")
                .font(.caption).foregroundColor(.secondary)
            Spacer()
            HStack {
                Button("Cancel") { dismiss() }.buttonStyle(Pill())
                Spacer()
                Button("Make it") {
                    dismiss()
                    let width = [360, 540, 720, 100_000][size]
                    model.saveAnimation(format: format == 0 ? "webp" : "gif", width: width, fps: [12, 20, 30][fps])
                }.buttonStyle(Pill(primary: true))
            }
        }
        .padding(20)
        .presentationDetents([.medium])
        .preferredColorScheme(.dark)
    }
}

/// A share code, as an item for `.sheet(item:)`.
struct SharedCode: Identifiable {
    let name: String
    let code: String
    var id: String { code }
}

/// The parts "Surprise me" keeps.
struct LocksSheet: View {
    @ObservedObject var model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationView {
            List {
                Section(footer: Text("Surprise me keeps these and takes the rest from presets at random. Back undoes a surprise.")) {
                    ForEach(Array(AppModel.parts.enumerated()), id: \.offset) { i, part in
                        Toggle(AppModel.partLabels[i], isOn: Binding(get: { model.locks.contains(part) }, set: { on in if on { model.locks.insert(part) } else { model.locks.remove(part) } }))
                    }
                }
            }
            .navigationTitle("Keep as they are")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                ToolbarItem(placement: .confirmationAction) { Button("Surprise me") { dismiss(); model.surprise() } }
            }
        }
    }
}

/// A scene's share code: its QR code to scan, and the text to copy or send.
struct CodeSheet: View {
    let code: SharedCode
    @Environment(\.dismiss) private var dismiss
    @State private var copied = false

    var body: some View {
        NavigationView {
            ScrollView {
                VStack(spacing: 16) {
                    if let img = CodeSheet.qr(code.code) {
                        Image(uiImage: img).interpolation(.none).resizable().scaledToFit()
                            .frame(maxWidth: 320).padding(12).background(Color.white).cornerRadius(12)
                    } else {
                        Text("Too long for a QR code; share the text instead.").font(.caption).foregroundColor(.secondary)
                    }
                    Text(code.code).font(.system(.caption, design: .monospaced)).textSelection(.enabled).lineLimit(8)
                    HStack {
                        Button(copied ? "Copied" : "Copy") { UIPasteboard.general.string = code.code; copied = true }.buttonStyle(Pill())
                        ShareLink(item: code.code) { Text("Send…") }.buttonStyle(Pill(primary: true))
                    }
                }
                .padding()
            }
            .navigationTitle(code.name)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
        }
    }

    /// The QR code of `text`, drawn by Core Image.
    static func qr(_ text: String) -> UIImage? {
        guard let f = CIFilter(name: "CIQRCodeGenerator") else { return nil }
        f.setValue(Data(text.utf8), forKey: "inputMessage")
        f.setValue("L", forKey: "inputCorrectionLevel")
        guard let out = f.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
              let cg = CIContext().createCGImage(out, from: out.extent) else { return nil }
        return UIImage(cgImage: cg)
    }
}
