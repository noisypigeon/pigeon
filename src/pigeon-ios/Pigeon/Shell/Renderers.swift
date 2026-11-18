// The four views of the view contract, drawn; see
// .claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md §1.

import PigeonCore
import SwiftUI

struct ValueText: View {
    let value: Value

    var body: some View {
        switch value {
        case .text(let t): Text(t)
        case .code(let t): Text(t).font(Theme.mono(13))
        case .sha(let t): Text(String(t.prefix(12))).font(Theme.mono(13))
        case .time(let at):
            Text(Date(timeIntervalSince1970: TimeInterval(at)), style: .relative)
        case .duration(let s): Text(Self.duration(s))
        case .count(let n): Text("\(n)")
        case .link(let text, let href):
            if let url = URL(string: href) { Link(text, destination: url) } else { Text(text) }
        case .progress(let stop, let of, let mark):
            VStack(alignment: .trailing, spacing: 2) {
                ProgressView(value: Double(stop), total: Double(max(of, 1)))
                    .tint(mark == .failed ? Theme.warn : Theme.good)
                Text("\(stop)/\(of)").font(Theme.mono(12)).foregroundStyle(Theme.muted)
            }
        case .series(let points):
            Text(points.map { "\($0.label) \($0.n)" }.joined(separator: " · "))
        case .unknown: Text("–").foregroundStyle(Theme.muted)
        }
    }

    static func duration(_ seconds: Int) -> String {
        let h = seconds / 3600
        let m = (seconds % 3600) / 60
        if h > 0 { return "\(h)h \(m)m" }
        if m > 0 { return "\(m)m \(seconds % 60)s" }
        return "\(seconds)s"
    }
}

struct FieldRows: View {
    let fields: [Field]

    var body: some View {
        ForEach(Array(fields.enumerated()), id: \.offset) { _, field in
            LabeledContent {
                ValueText(value: field.value)
            } label: {
                Text(field.label).foregroundStyle(Theme.muted)
            }
        }
    }
}

struct ActionButtons: View {
    let actions: [Action]
    let act: (Action) -> Void

    var body: some View {
        ForEach(Array(actions.enumerated()), id: \.offset) { _, action in
            ActionButton(action: action, act: act)
        }
    }
}

struct ActionButton: View {
    let action: Action
    let act: (Action) -> Void

    var body: some View {
        switch action.style {
        case .primary:
            Button { act(action) } label: { label }.buttonStyle(.borderedProminent)
        case .destructive:
            Button(role: .destructive) { act(action) } label: { label }.buttonStyle(.bordered)
        case .plain:
            Button { act(action) } label: { label }.buttonStyle(.bordered)
        }
    }

    private var label: some View {
        HStack(spacing: 6) {
            if action.proof { Image(systemName: "key.fill") }
            Text(action.label)
        }
    }
}

struct ListRenderer: View {
    let title: String
    let items: [Row]
    let act: (Action) -> Void

    var body: some View {
        List {
            ForEach(items) { row in
                Button {
                    if let open = row.open { act(open) }
                } label: {
                    HStack {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(row.title).foregroundStyle(Theme.ink)
                            if let subtitle = row.subtitle {
                                Text(subtitle).font(.footnote).foregroundStyle(Theme.muted)
                            }
                        }
                        Spacer()
                        if row.needs == .owner {
                            Text("needs you").font(.caption.bold()).foregroundStyle(Theme.accent)
                        } else if let badge = row.badge {
                            Text(badge).font(.caption).foregroundStyle(Theme.muted)
                        }
                    }
                }
                .disabled(row.open == nil)
            }
        }
        .navigationTitle(title)
    }
}

struct CardRenderer: View {
    let title: String
    let body_: String?
    let fields: [Field]
    let actions: [Action]
    let act: (Action) -> Void

    var body: some View {
        List {
            if let text = body_ { SwiftUI.Section { Text(text) } }
            SwiftUI.Section { FieldRows(fields: fields) }
            if !actions.isEmpty { SwiftUI.Section { ActionButtons(actions: actions, act: act) } }
        }
        .navigationTitle(title)
    }
}

struct DetailRenderer: View {
    let title: String
    let sections: [PigeonCore.Section]
    let actions: [Action]
    let act: (Action) -> Void

    var body: some View {
        List {
            ForEach(Array(sections.enumerated()), id: \.offset) { _, section in
                SwiftUI.Section(section.heading ?? "") {
                    FieldRows(fields: section.fields)
                    if let text = section.text { Text(text).font(Theme.mono(13)) }
                }
            }
            if !actions.isEmpty { SwiftUI.Section { ActionButtons(actions: actions, act: act) } }
        }
        .navigationTitle(title)
    }
}

struct FormRenderer: View {
    let title: String
    let fields: [Input]
    let submit: Action
    let act: (Action) -> Void
    @State private var text: [String: String] = [:]
    @State private var flags: [String: Bool] = [:]

    var body: some View {
        Form {
            ForEach(fields, id: \.key) { input in
                VStack(alignment: .leading, spacing: 4) {
                    control(input)
                    if let help = input.help {
                        Text(help).font(.footnote).foregroundStyle(Theme.muted)
                    }
                }
            }
            Button(submit.label) { act(collected()) }
                .buttonStyle(.borderedProminent)
        }
        .navigationTitle(title)
    }

    @ViewBuilder
    private func control(_ input: Input) -> some View {
        let binding = Binding(get: { text[input.key] ?? "" }, set: { text[input.key] = $0 })
        switch input.input {
        case .text: TextField(input.label, text: binding)
        case .number: TextField(input.label, text: binding).keyboardType(.numbersAndPunctuation)
        case .secret: SecureField(input.label, text: binding)
        case .toggle:
            Toggle(
                input.label,
                isOn: Binding(get: { flags[input.key] ?? false }, set: { flags[input.key] = $0 }))
        case .choice:
            Picker(input.label, selection: binding) {
                Text("–").tag("")
                ForEach(input.options ?? [], id: \.self) { Text($0).tag($0) }
            }
        }
    }

    private func collected() -> Action {
        var args = submit.arguments
        for input in fields {
            switch input.input {
            case .toggle: args[input.key] = .bool(flags[input.key] ?? false)
            case .number:
                let t = text[input.key] ?? ""
                args[input.key] = Double(t).map(JSONValue.number) ?? .string(t)
            default: args[input.key] = .string(text[input.key] ?? "")
            }
        }
        return Action(
            label: submit.label, tool: submit.tool, arguments: args, style: submit.style,
            proof: submit.proof)
    }
}

struct ViewModelView: View {
    let model: ViewModel
    let act: (Action) -> Void

    var body: some View {
        switch model {
        case .list(let title, let items, _):
            ListRenderer(title: title, items: items, act: act)
        case .card(let title, let body, let fields, let actions):
            CardRenderer(title: title, body_: body, fields: fields, actions: actions, act: act)
        case .detail(let title, let sections, let actions):
            DetailRenderer(title: title, sections: sections, actions: actions, act: act)
        case .form(let title, let fields, let submit):
            FormRenderer(title: title, fields: fields, submit: submit, act: act)
        }
    }
}
