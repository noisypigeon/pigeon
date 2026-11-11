import SwiftUI

/// A blank screen with "Pigeon" centred.
struct ContentView: View {
    var body: some View {
        Text("Pigeon")
            .font(.largeTitle)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

#Preview {
    ContentView()
}
