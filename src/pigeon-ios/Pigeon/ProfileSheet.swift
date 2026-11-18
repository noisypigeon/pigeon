// The profile: avatar, name, appearance, passkeys and devices; see the accounts, profiles and
// instances ADR §4.

import PhotosUI
import PigeonCore
import SwiftUI

struct ProfileSheet: View {
    @EnvironmentObject private var store: Store
    @State private var name = ""
    @State private var picked: PhotosPickerItem?
    @State private var proving: Proving?

    private var appearance: Appearance { store.me?.profile.appearance ?? .system }

    var body: some View {
        ZStack {
            Theme.background.ignoresSafeArea()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    avatar
                    TextField("Name", text: $name)
                        .padding(12)
                        .block()
                        .submitLabel(.done)
                        .onSubmit {
                            guard let valid = Profile.validName(name) else { return }
                            Task { _ = await store.rename(valid) }
                        }
                    Eyebrow(text: "Appearance")
                    appearanceRow
                    Eyebrow(text: "Passkeys")
                    passkeys
                    Eyebrow(text: "Devices")
                    devices
                    if let error = store.error {
                        Text(error).font(.footnote).foregroundStyle(Theme.accent)
                    }
                }
                .foregroundStyle(Theme.ink)
                .padding(20)
            }
        }
        .presentationDetents([.large])
        .sheet(item: $proving, onDismiss: { Task { await store.loadProfile() } }) { p in
            if let client = try? store.mcp() {
                ProofSheet(client: client, page: "", purpose: p.purpose, target: p.target)
            }
        }
        .onAppear { name = store.me?.profile.name ?? "" }
        .onChange(of: store.me?.profile.name) { _, new in name = new ?? name }
        .onChange(of: picked) { _, item in
            guard let item else { return }
            Task { await upload(item) }
        }
    }

    private var avatar: some View {
        HStack(spacing: 16) {
            PhotosPicker(selection: $picked, matching: .images) {
                Group {
                    if let data = store.avatar, let image = UIImage(data: data) {
                        Image(uiImage: image).resizable().scaledToFill()
                    } else {
                        Image("Avatar").resizable().scaledToFill()
                    }
                }
                .frame(width: 88, height: 88)
                .clipShape(Circle())
                .overlay(Circle().stroke(Theme.card, lineWidth: 2))
            }
            .accessibilityLabel("Change photo")
            if store.avatar != nil {
                Button("Remove photo") { Task { _ = await store.removeAvatar() } }
                    .foregroundStyle(Theme.accent)
            }
        }
    }

    private var appearanceRow: some View {
        HStack(spacing: 10) {
            ForEach(Appearance.allCases) { a in
                Button {
                    Task { _ = await store.setAppearance(a) }
                } label: {
                    Text(a.label)
                        .font(.subheadline.weight(.semibold))
                        .frame(maxWidth: .infinity, minHeight: 48)
                        .foregroundStyle(appearance == a ? Theme.onAccent : Theme.ink)
                        .background(
                            RoundedRectangle(cornerRadius: 12)
                                .fill(appearance == a ? Theme.accent : Theme.card)
                        )
                        .background(
                            RoundedRectangle(cornerRadius: 12).fill(Theme.shadow)
                                .offset(x: 3, y: 3))
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(appearance == a ? .isSelected : [])
            }
        }
    }

    private var passkeys: some View {
        let rows = store.me?.passkeys ?? []
        return ForEach(rows) { row in
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(row.name).font(.headline)
                    Text("Added \(date(row.created))").font(.footnote).foregroundStyle(Theme.muted)
                    Text("Last used \(row.used.map(date) ?? "never")")
                        .font(.footnote).foregroundStyle(Theme.muted)
                }
                Spacer()
                Button("Remove") { proving = Proving(purpose: "passkey-remove", target: row.id) }
                    .disabled(!canRemovePasskey(count: rows.count))
            }
            .padding(12)
            .block()
        }
    }

    private var devices: some View {
        ForEach(store.me?.devices ?? []) { row in
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(row.name).font(.headline)
                    if row.current {
                        Text("This iPhone").font(.footnote).foregroundStyle(Theme.accent)
                    }
                    Text("Last seen \(date(row.seen))").font(.footnote).foregroundStyle(Theme.muted)
                }
                Spacer()
                Button("Unlink") { proving = Proving(purpose: "unlink", target: row.id) }
            }
            .padding(12)
            .block()
        }
    }

    private func date(_ at: Int) -> String {
        Date(timeIntervalSince1970: TimeInterval(at)).formatted(date: .abbreviated, time: .omitted)
    }

    /// Crops the photo to a square, draws it at 256 pt, and keeps the first JPEG of at most 64 KiB.
    private func upload(_ item: PhotosPickerItem) async {
        defer { picked = nil }
        guard let data = try? await item.loadTransferable(type: Data.self),
            let image = UIImage(data: data)
        else {
            store.error = "That photo could not be read."
            return
        }
        let side = min(image.size.width, image.size.height)
        let origin = CGPoint(
            x: -(image.size.width - side) / 2 * 256 / side,
            y: -(image.size.height - side) / 2 * 256 / side)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let drawn = UIGraphicsImageRenderer(size: CGSize(width: 256, height: 256), format: format)
            .image { _ in
                image.draw(
                    in: CGRect(
                        origin: origin,
                        size: CGSize(
                            width: image.size.width * 256 / side,
                            height: image.size.height * 256 / side)))
            }
        for quality in [0.8, 0.5, 0.3] {
            if let jpeg = drawn.jpegData(compressionQuality: quality), jpeg.count <= 65536 {
                _ = await store.setAvatar(jpeg: jpeg)
                return
            }
        }
        store.error = "That photo is too large to use."
    }
}
