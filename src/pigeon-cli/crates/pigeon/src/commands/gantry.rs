//! `pigeon gantry`.

use super::SUCCESS;

/// Prints gantry's status.
#[must_use]
pub fn run() -> i32 {
    println!("{}", gantry::status());
    SUCCESS
}
