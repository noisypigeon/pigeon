// The sample page: print it blank or filled in, save its PDF, and scan it; nothing it scores is kept; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md, its sample amendment.

import PigeonCore
import SwiftUI
import UIKit

/// Trying every step on the sample page, with nothing kept.
struct LearnerSampleView: View {
    @ObservedObject var learner: LearnerStore
    @State private var scanning = false
    @State private var files: [URL] = []

    var body: some View {
        ZStack {
            DotGrid()
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    VStack(alignment: .leading, spacing: 2) {
                        Eyebrow(text: "Nothing is saved")
                        Text("Try a sample page").font(Theme.display(22)).foregroundStyle(Theme.ink)
                    }
                    Text(
                        "Print the sample, then scan it to try every step: the camera, checking the marks, the score and the study tips. Its questions are made up, and its score never reaches your progress."
                    )
                    .font(.callout)
                    .foregroundStyle(Theme.ink2)
                    Text("The filled-in sample is already marked, so it can be scanned straight from the printer.")
                        .font(.footnote)
                        .foregroundStyle(Theme.muted)
                    Button { printSample(filledIn: true) } label: { learnerPrimary("Print the filled-in sample") }
                    Button { printSample(filledIn: false) } label: { learnerSecondary("Print a blank sample to fill in") }
                    Button { scanning = true } label: { learnerSecondary("Scan the sample") }
                    if !files.isEmpty {
                        ShareLink(items: files) { learnerSecondary("Save or send the PDFs") }
                    }
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 16)
            }
        }
        .navigationTitle("Sample page")
        .navigationBarTitleDisplayMode(.inline)
        .task { files = Self.writeFiles() }
        .navigationDestination(isPresented: $scanning) { LearnerScanView(learner: learner) }
    }

    private func printSample(filledIn: Bool) {
        let info = UIPrintInfo.printInfo()
        info.outputType = .grayscale
        info.jobName = filledIn ? "Pigeon sample page, filled in" : "Pigeon sample page"
        let controller = UIPrintInteractionController.shared
        controller.printInfo = info
        controller.printingItem = LearnerPDF.renderSample(filledIn: filledIn)
        controller.present(animated: true)
    }

    /// Both sample PDFs, written to the temporary folder to share.
    private static func writeFiles() -> [URL] {
        var out: [URL] = []
        for filledIn in [true, false] {
            let name = filledIn ? "Pigeon sample page, filled in.pdf" : "Pigeon sample page, blank.pdf"
            let url = FileManager.default.temporaryDirectory.appendingPathComponent(name)
            if (try? LearnerPDF.renderSample(filledIn: filledIn).write(to: url, options: .atomic)) != nil {
                out.append(url)
            }
        }
        return out
    }
}
