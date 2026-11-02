//! The pigeon command-line tool.

fn main() {
    println!(
        "pigeon {} (sentinel {})",
        env!("CARGO_PKG_VERSION"),
        sentinel::version()
    );
}
