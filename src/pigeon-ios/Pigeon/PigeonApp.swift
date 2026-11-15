// The Pigeon app: links this iPhone, then shows its tabs; see
// .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md.

import PigeonCore
import SwiftUI
import UIKit
import UserNotifications

extension Appearance {
    var scheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }
}

final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    var onToken: ((Data) -> Void)?
    var onLink: ((URL) -> Void)?
    var pendingLink: URL?

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions options: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self
        return true
    }

    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        onToken?(deviceToken)
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
        withCompletionHandler done: @escaping () -> Void
    ) {
        if let link = response.notification.request.content.userInfo["link"] as? String,
            let url = URL(string: link)
        {
            if let onLink { onLink(url) } else { pendingLink = url }
        }
        done()
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification,
        withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        done([.banner, .sound])
    }
}

@main
struct PigeonApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var store = Store()
    @AppStorage("appearance") private var appearance = Appearance.system.rawValue
    @State private var link: URL?
    @State private var joining: JoinLink?
    /// The learner's instance, once this iPhone joined one; see the learner ADR's 2026-11-15 amendment.
    @AppStorage("learner.instance") private var learnerLink: Data?

    var body: some Scene {
        WindowGroup {
            Group {
                if store.linked {
                    RootView(link: $link)
                } else if learnerLink != nil {
                    LearnerHomeView()
                } else {
                    LinkView()
                }
            }
            .environmentObject(store)
            .preferredColorScheme(Appearance(rawValue: appearance)?.scheme)
            .onOpenURL(perform: open)
            .fullScreenCover(item: $joining) { LearnerJoinView(link: $0) }
            .task {
                delegate.onToken = { token in Task { await store.registerPush(token) } }
                delegate.onLink = { url in open(url) }
                if let url = delegate.pendingLink {
                    link = url
                    delegate.pendingLink = nil
                }
            }
            .onChange(of: store.linked) { _, linked in
                if linked { askForNotifications() }
            }
            .onAppear {
                if store.linked { askForNotifications() }
            }
        }
    }

    /// `pigeon://approval/<id>` opens that card; `pigeon://home` opens the Tracker;
    /// `pigeon://join?…` joins a learner's instance.
    private func open(_ url: URL) {
        guard url.scheme == "pigeon" else { return }
        if let join = JoinLink(url: url) {
            joining = join
            return
        }
        link = url
    }

    private func askForNotifications() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) {
            granted, _ in
            guard granted else { return }
            DispatchQueue.main.async { UIApplication.shared.registerForRemoteNotifications() }
        }
    }
}
