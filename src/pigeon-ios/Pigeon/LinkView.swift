// First run: link this iPhone to the Platform instance, or practise for the 7L test without an
// account (the learner ADR's step 2).

import SwiftUI

struct LinkView: View {
    @EnvironmentObject private var store: Store
    @State private var practising = false

    var body: some View {
        ZStack {
            DotGrid()
            VStack(alignment: .leading, spacing: 18) {
                Spacer()
                Image("Avatar")
                    .resizable()
                    .frame(width: 72, height: 72)
                    .clipShape(Circle())
                    .overlay(Circle().stroke(Theme.ink, lineWidth: 2))
                Text("Link this iPhone")
                    .font(Theme.display(32))
                    .foregroundStyle(Theme.ink)
                Text(
                    "You'll sign in to platform.pigeon.dev and prove it's you with your passkey or code. After that, this iPhone reads your insights and approves with Face ID."
                )
                .font(.body)
                .foregroundStyle(Theme.ink2)
                if let error = store.error {
                    Text(error).font(.footnote).foregroundStyle(Theme.accent)
                }
                Button {
                    Task { await store.link() }
                } label: {
                    Text(store.busy ? "Linking…" : "Link with Platform")
                        .font(.headline)
                        .frame(maxWidth: .infinity, minHeight: 52)
                        .foregroundStyle(Theme.onAccent)
                }
                .background(RoundedRectangle(cornerRadius: 12).fill(Theme.accent))
                .background(RoundedRectangle(cornerRadius: 12).fill(Theme.shadow).offset(x: 3, y: 3))
                .disabled(store.busy)
                Button {
                    practising = true
                } label: {
                    Text("Practise for the 7L test")
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 44)
                        .foregroundStyle(Theme.ink)
                }
                .background(RoundedRectangle(cornerRadius: 10).fill(Theme.chip))
                Spacer().frame(height: 24)
            }
            .padding(24)
        }
        .fullScreenCover(isPresented: $practising) { LearnerHomeView() }
    }
}
