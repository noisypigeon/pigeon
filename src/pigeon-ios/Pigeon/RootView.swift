// The two tabs, Now and Insights, and the bar above them; see
// .claude/decisions/2026-11-15-runtime-ios-app-now-tab.md §1.

import PigeonCore
import SwiftUI

enum AppTab: Hashable {
    case feed, now, insights
}

/// The instance chip and the avatar button.
struct TopBar: View {
    @EnvironmentObject private var store: Store
    @Binding var sheet: Sheet?

    var body: some View {
        HStack {
            Button {
                sheet = .instances
            } label: {
                HStack(spacing: 8) {
                    Circle().fill(Theme.accent).frame(width: 8, height: 8)
                    Text("Platform").font(.headline)
                    Image(systemName: "chevron.down")
                }
                .foregroundStyle(Theme.ink)
                .padding(.horizontal, 12)
                .frame(minHeight: 40)
                .block(radius: 20)
            }
            .accessibilityLabel("Instance: Platform. Switch instance")
            Spacer()
            Button {
                sheet = .profile
            } label: {
                avatarImage
                    .resizable()
                    .scaledToFill()
                    .frame(width: 40, height: 40)
                    .clipShape(Circle())
                    .overlay(Circle().stroke(Theme.card, lineWidth: 2))
                    .background(Circle().fill(Theme.shadow).offset(x: 2, y: 2))
            }
            .accessibilityLabel("Profile")
        }
        .padding(.top, 8)
    }

    private var avatarImage: Image {
        if let data = store.avatar, let image = UIImage(data: data) { return Image(uiImage: image) }
        return Image("Avatar")
    }
}

struct RootView: View {
    @EnvironmentObject private var store: Store
    @Environment(\.scenePhase) private var scenePhase
    @Binding var link: URL?
    @State private var tab: AppTab = .now
    @State private var sheet: Sheet?
    @State private var openApproval: String?

    var body: some View {
        TabView(selection: $tab) {
            ShellView()
                .tabItem { Label("Feed", systemImage: "tray") }
                .tag(AppTab.feed)
            NowView(sheet: $sheet)
                .tabItem { Label("Now", systemImage: "point.3.connected.trianglepath.dotted") }
                .badge(needsMe(home: store.home, tracker: store.tracker).count)
                .tag(AppTab.now)
            InsightsView(sheet: $sheet)
                .tabItem { Label("Insights", systemImage: "chart.bar") }
                .tag(AppTab.insights)
        }
        .tint(Theme.accent)
        .task {
            await store.refresh()
            await store.loadProfile()
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active {
                Task {
                    await store.refresh()
                    await store.loadTracker()
                }
            }
        }
        .onChange(of: link, initial: true) { _, url in
            guard let url, url.scheme == "pigeon" else { return }
            if url.host == "approval", let id = url.pathComponents.dropFirst().first {
                tab = .now
                sheet = .approval(id)
            } else if url.host == "home" {
                tab = .now
            }
            link = nil
        }
        .sheet(item: $sheet) { which in
            switch which {
            case .instances: InstancesSheet()
            case .profile: ProfileSheet()
            case .chart(let metric): ChartSheet(metric: metric)
            case .approval(let id): ApprovalSheet(id: id)
            }
        }
    }
}
