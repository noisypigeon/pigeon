// Scanning a printed page with the back camera, and checking its marks before they are scored; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §4 and step 3.

import AVFoundation
import PigeonCore
import SwiftUI

/// Reads the camera's frames until two in a row read the same page; the frames never leave the phone.
final class PageScanner: NSObject, ObservableObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    @Published var reading: PageReading?
    @Published var denied = false
    let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "learner.scan")
    private let circles: [String: [PlacedCircle]]
    private var last: PageReading?
    private var done = false

    /// `circles` is LearnerStore's, keyed as its `pageKey`.
    init(circles: [String: [PlacedCircle]]) {
        self.circles = circles
        super.init()
    }

    func start() {
        reading = nil
        AVCaptureDevice.requestAccess(for: .video) { ok in
            DispatchQueue.main.async { self.denied = !ok }
            guard ok else { return }
            self.queue.async {
                self.configure()
                self.last = nil
                self.done = false
                self.session.startRunning()
            }
        }
    }

    func stop() { queue.async { self.session.stopRunning() } }

    private func configure() {
        guard session.inputs.isEmpty,
            let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back),
            let input = try? AVCaptureDeviceInput(device: device)
        else { return }
        session.beginConfiguration()
        session.sessionPreset = .hd1920x1080
        if session.canAddInput(input) { session.addInput(input) }
        let output = AVCaptureVideoDataOutput()
        output.videoSettings = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange
        ]
        output.alwaysDiscardsLateVideoFrames = true
        output.setSampleBufferDelegate(self, queue: queue)
        if session.canAddOutput(output) { session.addOutput(output) }
        session.commitConfiguration()
    }

    func captureOutput(
        _ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection
    ) {
        guard !done, let buffer = CMSampleBufferGetImageBuffer(sampleBuffer), let image = Self.luma(buffer)
        else { return }
        guard let read = readPage(image, circles: { self.circles["\($0)-\($1)"] }) else {
            last = nil
            return
        }
        if let last, last.form == read.form, last.page == read.page, last.marks == read.marks {
            done = true
            session.stopRunning()
            DispatchQueue.main.async { self.reading = read }
        }
        last = read
    }

    /// The frame's brightness plane, which is already greyscale.
    static func luma(_ buffer: CVPixelBuffer) -> GrayImage? {
        CVPixelBufferLockBaseAddress(buffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(buffer, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddressOfPlane(buffer, 0) else { return nil }
        let w = CVPixelBufferGetWidthOfPlane(buffer, 0), h = CVPixelBufferGetHeightOfPlane(buffer, 0)
        let row = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0)
        let src = base.assumingMemoryBound(to: UInt8.self)
        var pixels = [UInt8](repeating: 0, count: w * h)
        pixels.withUnsafeMutableBufferPointer { dst in
            for y in 0..<h { memcpy(dst.baseAddress! + y * w, src + y * row, w) }
        }
        return GrayImage(width: w, height: h, pixels: pixels)
    }
}

/// The camera's live picture.
struct CameraPreview: UIViewRepresentable {
    let session: AVCaptureSession

    final class PreviewView: UIView {
        override class var layerClass: AnyClass { AVCaptureVideoPreviewLayer.self }
        var preview: AVCaptureVideoPreviewLayer { layer as! AVCaptureVideoPreviewLayer }
    }

    func makeUIView(context: Context) -> PreviewView {
        let view = PreviewView()
        view.preview.session = session
        view.preview.videoGravity = .resizeAspectFill
        return view
    }

    func updateUIView(_ view: PreviewView, context: Context) {}
}

/// The camera, with how to hold the page; a steady reading opens the check screen.
struct LearnerScanView: View {
    @ObservedObject var learner: LearnerStore
    @StateObject private var scanner: PageScanner
    @State private var reviewing = false

    init(learner: LearnerStore) {
        self.learner = learner
        _scanner = StateObject(wrappedValue: PageScanner(circles: learner.circles))
    }

    var body: some View {
        ZStack(alignment: .bottom) {
            CameraPreview(session: scanner.session).ignoresSafeArea()
            VStack(alignment: .leading, spacing: 6) {
                Text(scanner.denied ? "Camera access is off" : "Fit the four corner squares in view")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Theme.ink)
                Text(
                    scanner.denied
                        ? "Turn it on for Pigeon in Settings, or enter your answers instead."
                        : "Lay the page flat on a plain, light surface and hold still. The picture stays on this iPhone."
                )
                .font(.footnote)
                .foregroundStyle(Theme.muted)
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .block()
            .padding(16)
        }
        .navigationTitle("Scan a page")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { scanner.start() }
        .onDisappear { scanner.stop() }
        .onChange(of: scanner.reading) { _, r in reviewing = r != nil }
        .navigationDestination(isPresented: $reviewing) {
            if let r = scanner.reading { ScanReviewView(learner: learner, reading: r) }
        }
    }
}

/// The marks as read, to correct before scoring; light marks are shown in orange to check.
struct ScanReviewView: View {
    @ObservedObject var learner: LearnerStore
    let reading: PageReading
    @State private var marks: [Set<Int>]
    @State private var result: Attempt?
    @State private var previousBest: Int?
    @State private var showResult = false

    init(learner: LearnerStore, reading: PageReading) {
        self.learner = learner
        self.reading = reading
        _marks = State(initialValue: reading.marks.map { Set($0) })
    }

    var body: some View {
        ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Eyebrow(text: formLabel(reading.form) + " · page \(reading.page + 1)")
                    Text("Check the marks").font(Theme.display(22)).foregroundStyle(Theme.ink)
                    Text(
                        reading.unclear.isEmpty
                            ? "Tap a letter to change it."
                            : "Light marks to check are in orange. Tap a letter to change it."
                    )
                    .font(.footnote)
                    .foregroundStyle(Theme.muted)
                    ForEach(marks.indices, id: \.self) { i in row(i) }
                    Button { scoreIt() } label: { learnerPrimary("Score it") }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
            }
        }
        .navigationDestination(isPresented: $showResult) {
            if let result {
                ResultView(learner: learner, attempt: result, previousBest: previousBest)
            }
        }
    }

    private func row(_ i: Int) -> some View {
        let number = reading.page * questionsPerPage + i + 1
        let check = reading.unclear.contains(i)
        return HStack(spacing: 8) {
            Text("\(number)")
                .font(Theme.mono(13, .semibold))
                .foregroundStyle(check ? Theme.kind(1) : Theme.ink)
                .frame(width: 30, alignment: .trailing)
            ForEach(0..<4, id: \.self) { k in
                let on = marks[i].contains(k)
                Button {
                    if on { marks[i].remove(k) } else { marks[i].insert(k) }
                } label: {
                    Text(choiceLetters[k])
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .foregroundStyle(on ? Theme.onAccent : Theme.ink)
                        .background(
                            RoundedRectangle(cornerRadius: 10)
                                .fill(on ? Theme.accent : check ? Theme.kind(1).opacity(0.18) : Theme.chip))
                }
                .accessibilityLabel("Question \(number), \(choiceLetters[k])\(on ? ", marked" : "")")
            }
        }
    }

    private func scoreIt() {
        previousBest = learner.best
        result = learner.record(form: reading.form, page: reading.page, marks: marks.map { $0.sorted() })
        showResult = result != nil
    }
}
