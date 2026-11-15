// The learner's printed tests: a form's pages as one US Letter PDF, drawn from PigeonCore's layout; see
// .claude/decisions/2026-11-15-runtime-learner-practice-standalone-app.md §3 and its 2026-11-15 amendment.

import PigeonCore
import UIKit

enum LearnerPDF {
    /// Points per millimetre.
    static let pt = 72 / 25.4
    static let pagePoints = CGSize(width: 612, height: 792)

    private static let ink = UIColor(white: 0.07, alpha: 1)
    private static let mid = UIColor(white: 0.33, alpha: 1)
    private static let light = UIColor(white: 0.55, alpha: 1)
    private static let boxLine = UIColor(white: 0.6, alpha: 1)
    private static let pen = UIColor(white: 0.16, alpha: 1)

    static func font(_ size: Double, bold: Bool) -> UIFont {
        UIFont.systemFont(ofSize: size * pt, weight: bold ? .bold : .regular)
    }

    /// PigeonCore's `Wrap`, measured in the font the PDF draws with.
    static func wrap(_ text: String, _ size: Double, _ bold: Bool, _ width: Double) -> [String] {
        let attrs: [NSAttributedString.Key: Any] = [.font: font(size, bold: bold)]
        let limit = width * pt
        var lines: [String] = []
        var cur = ""
        for word in text.split(separator: " ") {
            let candidate = cur.isEmpty ? String(word) : cur + " " + word
            if cur.isEmpty || (candidate as NSString).size(withAttributes: attrs).width <= limit {
                cur = candidate
            } else {
                lines.append(cur)
                cur = String(word)
            }
        }
        if !cur.isEmpty { lines.append(cur) }
        return lines
    }

    /// The form's pages as one PDF, and each page's circles as they were laid out.
    static func render(form: Form, bank: [Question]) -> (data: Data, circles: [[PlacedCircle]]) {
        let byID = Dictionary(bank.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        let pages = max(1, (form.questionIDs.count + questionsPerPage - 1) / questionsPerPage)
        var circles: [[PlacedCircle]] = []
        let renderer = UIGraphicsPDFRenderer(bounds: CGRect(origin: .zero, size: pagePoints))
        let data = renderer.pdfData { pdf in
            for page in 0..<pages {
                pdf.beginPage()
                let ids = pageSlice(form, page: page).ids
                let layout = layoutPage(
                    ids.compactMap { byID[$0] }, first: page * questionsPerPage, wrap: wrap)
                circles.append(layout.circles)
                draw(page: page, of: pages, form: form, count: ids.count, layout: layout, in: pdf.cgContext)
            }
        }
        return (data, circles)
    }

    /// The sample page as a PDF: blank to fill by hand, or `filledIn` with its marks already made.
    static func renderSample(filledIn: Bool) -> Data {
        let renderer = UIGraphicsPDFRenderer(bounds: CGRect(origin: .zero, size: pagePoints))
        return renderer.pdfData { pdf in
            pdf.beginPage()
            let c = pdf.cgContext
            draw(page: 0, of: 1, form: sampleForm, count: questionsPerPage, layout: sampleLayout, in: c)
            if filledIn {
                for circle in sampleFills { filled(c, circle.x, circle.y) }
            }
        }
    }

    private static func rect(_ b: PageBox) -> CGRect {
        CGRect(x: b.x * pt, y: b.y * pt, width: b.w * pt, height: b.h * pt)
    }

    /// Draws `s` with its baseline at `y` mm; `trailing` ends it at `x`, `centered` centres it there.
    private static func text(
        _ s: String, x: Double, y: Double, size: Double, bold: Bool = false, color: UIColor = ink,
        trailing: Bool = false, centered: Bool = false
    ) {
        let f = font(size, bold: bold)
        let a = NSAttributedString(string: s, attributes: [.font: f, .foregroundColor: color])
        let w = a.size().width
        var px = x * pt
        if trailing { px -= w } else if centered { px -= w / 2 }
        a.draw(at: CGPoint(x: px, y: y * pt - f.ascender))
    }

    private static func stroke(
        _ c: CGContext, _ r: CGRect, _ color: UIColor, _ widthMM: Double, radius: Double = 0
    ) {
        c.setStrokeColor(color.cgColor)
        c.setLineWidth(widthMM * pt)
        c.addPath(UIBezierPath(roundedRect: r, cornerRadius: radius * pt).cgPath)
        c.strokePath()
    }

    private static func line(
        _ c: CGContext, _ x1: Double, _ y1: Double, _ x2: Double, _ y2: Double, _ color: UIColor,
        _ widthMM: Double
    ) {
        c.setStrokeColor(color.cgColor)
        c.setLineWidth(widthMM * pt)
        c.setLineCap(.round)
        c.move(to: CGPoint(x: x1 * pt, y: y1 * pt))
        c.addLine(to: CGPoint(x: x2 * pt, y: y2 * pt))
        c.strokePath()
    }

    private static func bubble(_ c: CGContext, _ x: Double, _ y: Double, _ letter: String) {
        let r = LearnerPage.circleRadius
        let oval = CGRect(x: (x - r) * pt, y: (y - r) * pt, width: 2 * r * pt, height: 2 * r * pt)
        c.setFillColor(UIColor.white.cgColor)
        c.fillEllipse(in: oval)
        c.setStrokeColor(mid.cgColor)
        c.setLineWidth(0.28 * pt)
        c.strokeEllipse(in: oval)
        text(letter, x: x, y: y + r * 0.39, size: r * 1.08, bold: true, color: light, centered: true)
    }

    private static func filled(_ c: CGContext, _ x: Double, _ y: Double) {
        let r = LearnerPage.circleRadius - 0.12
        c.setFillColor(pen.cgColor)
        c.fillEllipse(in: CGRect(x: (x - r) * pt, y: (y - r) * pt, width: 2 * r * pt, height: 2 * r * pt))
    }

    /// "Like this": a full fill; "Not": a check, a cross, a dot and a half fill.
    private static func examples(_ c: CGContext, y: Double) {
        text("Like this", x: 128, y: y + 1.3, size: 2.3, bold: true, color: mid)
        bubble(c, 141.5, y, "B")
        filled(c, 141.5, y)
        text("Not", x: 147, y: y + 1.3, size: 2.3, bold: true, color: mid)
        let r = LearnerPage.circleRadius, k = r / 2.5
        for j in 0..<4 {
            let x = 156 + Double(j) * 5.3
            bubble(c, x, y, choiceLetters[j])
            switch j {
            case 0:
                c.setStrokeColor(pen.cgColor)
                c.setLineWidth(0.45 * pt)
                c.setLineCap(.round)
                c.setLineJoin(.round)
                c.move(to: CGPoint(x: (x - 1.8 * k) * pt, y: y * pt))
                c.addLine(to: CGPoint(x: (x - 0.5 * k) * pt, y: (y + 1.6 * k) * pt))
                c.addLine(to: CGPoint(x: (x + 2.1 * k) * pt, y: (y - 1.6 * k) * pt))
                c.strokePath()
            case 1:
                let d = r * 0.64
                line(c, x - d, y - d, x + d, y + d, pen, 0.45)
                line(c, x - d, y + d, x + d, y - d, pen, 0.45)
            case 2:
                c.setFillColor(pen.cgColor)
                c.fillEllipse(
                    in: CGRect(x: (x - 0.55) * pt, y: (y - 0.55) * pt, width: 1.1 * pt, height: 1.1 * pt))
            default:
                let rr = (r - 0.12) * pt
                c.setFillColor(pen.cgColor)
                c.move(to: CGPoint(x: x * pt, y: y * pt - rr))
                c.addArc(
                    center: CGPoint(x: x * pt, y: y * pt), radius: rr, startAngle: -.pi / 2,
                    endAngle: .pi / 2, clockwise: true)
                c.closePath()
                c.fillPath()
            }
        }
    }

    private static func draw(
        page: Int, of pages: Int, form: Form, count: Int, layout: PageLayout, in c: CGContext
    ) {
        c.setFillColor(ink.cgColor)
        for a in LearnerPage.anchors { c.fill(rect(a)) }
        for (b, on) in codeBits(form: form.number, page: page).enumerated() {
            let cell = rect(LearnerPage.codeCell(b))
            if on {
                c.setFillColor(ink.cgColor)
                c.fill(cell)
            } else {
                stroke(c, cell.insetBy(dx: 0.1 * pt, dy: 0.1 * pt), UIColor(white: 0.79, alpha: 1), 0.18)
            }
        }
        stroke(c, rect(LearnerPage.codeFrame), ink, 0.25)
        let sample = isSample(form.number)
        text(sample ? "SAMPLE PAGE" : "BC Class 7L Practice Test", x: 20, y: 14.6, size: 4.4, bold: true)
        let first = page * questionsPerPage + 1
        let subtitle =
            sample
            ? "Not a real test  ·  nothing is saved"
            : pages > 1
                ? "Questions \(first)–\(first + count - 1)  ·  Page \(page + 1) of \(pages)"
                : "\(count) questions"
        text(subtitle, x: 105, y: 14.6, size: 2.4, color: mid)
        text(String(format: "F%03d", form.number), x: 165.5, y: 14.3, size: 2.6, bold: true, trailing: true)
        text("Date", x: 20, y: 23.6, size: 2.5, bold: true)
        stroke(c, rect(PageBox(x: 28, y: 19.5, w: 30, h: 5.6)), boxLine, 0.25, radius: 0.5)
        text("Score", x: 62, y: 23.6, size: 2.5, bold: true)
        stroke(c, rect(PageBox(x: 71.5, y: 19.5, w: 12, h: 5.6)), boxLine, 0.25, radius: 0.5)
        text("/ \(count)   ·   pass \(passMark(count))", x: 85.5, y: 23.6, size: 2.5, color: mid)
        examples(c, y: 22.3)
        text(
            "One answer per question. Fill the circle completely with a dark pencil or pen.", x: 20,
            y: 30.1, size: 2.4, color: mid)
        line(c, 18, 32, LearnerPage.width - 18, 32, ink, 0.3)
        line(c, 108, LearnerPage.top - 1, 108, LearnerPage.bottom, UIColor(white: 0.83, alpha: 1), 0.22)
        for t in layout.texts {
            text(t.text, x: t.x, y: t.y, size: t.size, bold: t.bold, trailing: t.trailing)
        }
        for circle in layout.circles { bubble(c, circle.x, circle.y, choiceLetters[circle.choice]) }
        if sample {
            text(
                "Sample questions only, not the knowledge test. A score from this page is never saved.",
                x: LearnerPage.width / 2, y: 268.6, size: 2.4, color: mid, centered: true)
        }
    }
}
