// The Pigeon app: links this iPhone, then shows Home; see
// .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md.

import SwiftUI
import UIKit
import UserNotifications

/// Where the owner set Appearance: following the device unless they chose.
enum Appearance: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }
    var label: String { rawValue.capitalized }
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
            onLink?(url)
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
    @State private var openApproval: String?

    var body: some Scene {
        WindowGroup {
            Group {
                if store.linked {
                    HomeView(openApproval: $openApproval)
                } else {
                    LinkView()
                }
            }
            .environmentObject(store)
            .preferredColorScheme(Appearance(rawValue: appearance)?.scheme)
            .onOpenURL(perform: open)
            .task {
                delegate.onToken = { token in Task { await store.registerPush(token) } }
                delegate.onLink = { url in open(url) }
            }
            .onChange(of: store.linked) { _, linked in
                if linked { askForNotifications() }
            }
            .onAppear {
                if store.linked { askForNotifications() }
            }
        }
    }

    /// `pigeon://approval/<id>` opens that card; anything else opens Home.
    private func open(_ url: URL) {
        guard url.scheme == "pigeon", url.host == "approval" else { return }
        openApproval = url.pathComponents.dropFirst().first
    }

    private func askForNotifications() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) {
            granted, _ in
            guard granted else { return }
            DispatchQueue.main.async { UIApplication.shared.registerForRemoteNotifications() }
        }
    }
}
