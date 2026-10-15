import AVFoundation
import CoreMedia
import ImageIO
import Photos
import UIKit

/// Saving the scene to Photos: a still, a Live Photo or a looping video. iPhone apps cannot set the
/// wallpaper themselves; from Photos, Share → Use as Wallpaper does it (a Live Photo moves on the
/// lock screen, on iOS 17 and later).
extension AppModel {
    private static let howTo = "Open Photos, tap it, then Share → Use as Wallpaper."

    /// The screen's size in pixels, portrait, even (as video wants).
    private func screenPixels() -> (Int, Int) {
        let b = UIScreen.main.nativeBounds
        let w = Int(min(b.width, b.height)), h = Int(max(b.width, b.height))
        return (w & ~1, h & ~1)
    }

    private func photos(_ then: @escaping () -> Void) {
        PHPhotoLibrary.requestAuthorization(for: .addOnly) { s in
            DispatchQueue.main.async {
                if s == .authorized || s == .limited { then() } else {
                    self.note = Note(title: "PathForge can't add to Photos", text: "Allow it in Settings → PathForge → Photos.")
                }
            }
        }
    }

    /// Run `job` on the work queue with the preview paused and a busy message up; `job` reports
    /// progress (0..1) and returns a note to show when it is done.
    private func run(_ message: String, _ job: @escaping (@escaping (Double) -> Void) throws -> Note) {
        busy = message
        progress = nil
        preview?.pause()
        work.async {
            let note: Note
            do {
                note = try job { p in DispatchQueue.main.async { self.progress = p } }
            } catch {
                note = Note(title: "That didn't work", text: error.localizedDescription)
            }
            DispatchQueue.main.async {
                self.busy = nil
                self.progress = nil
                self.note = note
                self.preview?.resume()
            }
        }
    }

    func saveStill() {
        let scene = self.scene
        let now = preview?.now ?? (0, 0)
        let (w, h) = screenPixels()
        photos {
            self.run("Drawing it at full size…") { _ in
                guard let s = Gpu.open(scene) else { throw ExportError(Native.lastError) }
                guard let cg = s.image(distance: now.0, time: now.1, width: w, height: h) else { throw ExportError(Native.lastError) }
                Gpu.rendered(s)
                try Self.save { PHAssetCreationRequest.creationRequestForAsset(from: UIImage(cgImage: cg)) }
                return Note(title: "Saved to Photos", text: Self.howTo)
            }
        }
    }

    /// A Live Photo: three seconds of the walk and its first frame as the photo.
    func saveLivePhoto() {
        let scene = self.scene
        let start = preview?.now ?? (0, 0)
        let (w, h) = screenPixels()
        photos {
            self.run("Making a Live Photo…") { progress in
                guard let s = Gpu.open(scene) else { throw ExportError(Native.lastError) }
                let loop = s.loop
                let id = UUID().uuidString
                let dir = FileManager.default.temporaryDirectory
                let mov = dir.appendingPathComponent("\(id).mov"), jpg = dir.appendingPathComponent("\(id).jpg")
                defer { try? FileManager.default.removeItem(at: mov); try? FileManager.default.removeItem(at: jpg) }
                let fps = 30, frames = 90
                let speed = loop.length / loop.seconds
                var first: CGImage?
                try Self.writeVideo(to: mov, width: w, height: h, fps: fps, frames: frames, liveID: id) { i in
                    let t = start.1 + Float(i) / Float(fps)
                    guard let data = s.render(distance: start.0 + speed * Float(i) / Float(fps), time: t.truncatingRemainder(dividingBy: loop.seconds), width: w, height: h) else { throw ExportError(Native.lastError) }
                    if i == 0 { first = PathScene.cgImage(data, width: w, height: h); Gpu.rendered(s) }
                    progress(Double(i + 1) / Double(frames))
                    return data
                }
                guard let still = first, Self.writeJPEG(still, to: jpg, liveID: id) else { throw ExportError("Could not write the photo") }
                try Self.save {
                    let r = PHAssetCreationRequest.forAsset()
                    r.addResource(with: .photo, fileURL: jpg, options: nil)
                    r.addResource(with: .pairedVideo, fileURL: mov, options: nil)
                }
                return Note(title: "Live Photo saved", text: Self.howTo + " On the lock screen it moves when the phone wakes (iOS 17 and later).")
            }
        }
    }

    /// The whole loop as a video that joins end to start.
    func saveVideo() {
        let scene = self.scene
        let (w, h) = screenPixels()
        photos {
            self.run("Making a looping video…") { progress in
                guard let s = Gpu.open(scene) else { throw ExportError(Native.lastError) }
                let loop = s.loop
                let fps = 30
                let frames = max(1, Int((loop.seconds * Float(fps)).rounded()))
                let mov = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).mov")
                defer { try? FileManager.default.removeItem(at: mov) }
                try Self.writeVideo(to: mov, width: w, height: h, fps: fps, frames: frames, liveID: nil) { i in
                    // Frame i is i / frames of the way round the loop, in distance and in time.
                    let k = Float(i) / Float(frames)
                    guard let data = s.render(distance: loop.length * k, time: loop.seconds * k, width: w, height: h) else { throw ExportError(Native.lastError) }
                    if i == 0 { Gpu.rendered(s) }
                    progress(Double(i + 1) / Double(frames))
                    return data
                }
                try Self.save { PHAssetCreationRequest.forAsset().addResource(with: .video, fileURL: mov, options: nil) }
                return Note(title: "Video saved to Photos", text: "It loops seamlessly: play it on repeat.")
            }
        }
    }

    // ── Writing files ────────────────────────────────────────────────────

    private static func save(_ changes: @escaping () -> Void) throws {
        var failure: Error?
        let done = DispatchSemaphore(value: 0)
        PHPhotoLibrary.shared().performChanges(changes) { ok, err in
            if !ok { failure = err ?? ExportError("Photos refused it") }
            done.signal()
        }
        done.wait()
        if let f = failure { throw f }
    }

    /// A JPEG; with `liveID`, marked as the photo of a Live Photo.
    static func writeJPEG(_ img: CGImage, to url: URL, liveID: String?) -> Bool {
        guard let dest = CGImageDestinationCreateWithURL(url as CFURL, "public.jpeg" as CFString, 1, nil) else { return false }
        var props: [CFString: Any] = [kCGImageDestinationLossyCompressionQuality: 0.95]
        if let id = liveID { props[kCGImagePropertyMakerAppleDictionary] = ["17": id] }
        CGImageDestinationAddImage(dest, img, props as CFDictionary)
        return CGImageDestinationFinalize(dest)
    }

    /// An H.264 video of `frames` RGBA frames from `frame`; with `liveID`, marked as the video of a
    /// Live Photo (its content identifier, and the still image's time at the start).
    static func writeVideo(to url: URL, width: Int, height: Int, fps: Int, frames: Int, liveID: String?, frame: (Int) throws -> Data) throws {
        let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
        let settings: [String: Any] = [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: width,
            AVVideoHeightKey: height,
            AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: width * height * 6],
        ]
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
        input.expectsMediaDataInRealTime = false
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
        ])
        writer.add(input)
        var meta: AVAssetWriterInputMetadataAdaptor?
        if let id = liveID {
            let item = AVMutableMetadataItem()
            item.keySpace = .quickTimeMetadata
            item.key = "com.apple.quicktime.content.identifier" as NSString
            item.value = id as NSString
            item.dataType = "com.apple.metadata.datatype.UTF-8"
            writer.metadata = [item]
            let spec: [String: Any] = [
                kCMMetadataFormatDescriptionMetadataSpecificationKey_Identifier as String: "mdta/com.apple.quicktime.still-image-time",
                kCMMetadataFormatDescriptionMetadataSpecificationKey_DataType as String: "com.apple.metadata.datatype.int8",
            ]
            var desc: CMFormatDescription?
            CMMetadataFormatDescriptionCreateWithMetadataSpecifications(allocator: kCFAllocatorDefault, metadataType: kCMMetadataFormatType_Boxed,
                                                                        metadataSpecifications: [spec] as CFArray, formatDescriptionOut: &desc)
            let mi = AVAssetWriterInput(mediaType: .metadata, outputSettings: nil, sourceFormatHint: desc)
            meta = AVAssetWriterInputMetadataAdaptor(assetWriterInput: mi)
            writer.add(mi)
        }
        guard writer.startWriting() else { throw writer.error ?? ExportError("Could not start the video") }
        writer.startSession(atSourceTime: .zero)
        if let meta = meta {
            let item = AVMutableMetadataItem()
            item.keySpace = AVMetadataKeySpace(rawValue: "mdta")
            item.key = "com.apple.quicktime.still-image-time" as NSString
            item.value = 0 as NSNumber
            item.dataType = "com.apple.metadata.datatype.int8"
            meta.append(AVTimedMetadataGroup(items: [item], timeRange: CMTimeRange(start: .zero, duration: CMTime(value: 200, timescale: 3000))))
        }
        for i in 0..<frames {
            let rgba = try frame(i)
            while !input.isReadyForMoreMediaData { Thread.sleep(forTimeInterval: 0.005) }
            guard let pool = adaptor.pixelBufferPool else { throw writer.error ?? ExportError("No video buffers") }
            var made: CVPixelBuffer?
            CVPixelBufferPoolCreatePixelBuffer(nil, pool, &made)
            guard let pb = made else { throw ExportError("No video buffer") }
            CVPixelBufferLockBaseAddress(pb, [])
            if let base = CVPixelBufferGetBaseAddress(pb) {
                let dst = base.assumingMemoryBound(to: UInt8.self)
                let row = CVPixelBufferGetBytesPerRow(pb)
                rgba.withUnsafeBytes { raw in
                    guard let src = raw.bindMemory(to: UInt8.self).baseAddress else { return }
                    // RGBA to BGRA.
                    for y in 0..<height {
                        let s = src + y * width * 4, d = dst + y * row
                        for x in 0..<width {
                            d[x * 4] = s[x * 4 + 2]
                            d[x * 4 + 1] = s[x * 4 + 1]
                            d[x * 4 + 2] = s[x * 4]
                            d[x * 4 + 3] = 255
                        }
                    }
                }
            }
            CVPixelBufferUnlockBaseAddress(pb, [])
            if !adaptor.append(pb, withPresentationTime: CMTime(value: CMTimeValue(i), timescale: CMTimeScale(fps))) {
                throw writer.error ?? ExportError("Could not add a frame")
            }
        }
        input.markAsFinished()
        meta?.assetWriterInput.markAsFinished()
        let done = DispatchSemaphore(value: 0)
        writer.finishWriting { done.signal() }
        done.wait()
        if writer.status != .completed { throw writer.error ?? ExportError("Could not finish the video") }
    }
}

struct ExportError: LocalizedError {
    let text: String
    init(_ text: String) { self.text = text.isEmpty ? "Something went wrong" : text }
    var errorDescription: String? { text }
}
