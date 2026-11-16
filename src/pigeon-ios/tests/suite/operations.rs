//! The Swift names only operations in the platform's catalog.

use std::collections::BTreeSet;

fn catalog() -> BTreeSet<&'static str> {
    agent::wire::catalog().iter().map(|o| o.name).collect()
}

#[test]
fn every_operation_the_swift_names_is_in_the_catalog() {
    let names = catalog();
    let domains: BTreeSet<&str> = names
        .iter()
        .filter_map(|n| n.split_once('.'))
        .map(|(d, _)| d)
        .collect();
    let domains: Vec<&str> = domains.into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut unknown = Vec::new();
    for dir in pigeon_ios::SWIFT_DIRS {
        for file in pigeon_ios::swift_files(&pigeon_ios::root().join(dir)).expect("swift files") {
            let text = std::fs::read_to_string(&file).expect("read");
            for op in pigeon_ios::operation_literals(&text, &domains) {
                if names.contains(op) {
                    seen.insert(op.to_string());
                } else {
                    unknown.push(format!(
                        "{}: {op}",
                        file.strip_prefix(pigeon_ios::root())
                            .unwrap_or(&file)
                            .display()
                    ));
                }
            }
        }
    }
    assert!(unknown.is_empty(), "not in the catalog: {unknown:#?}");
    // The approval sheet's operations: a scan that finds none reads nothing.
    for op in [
        "change.authorize",
        "change.approve",
        "change.merge",
        "change.close",
    ] {
        assert!(seen.contains(op), "{op} wasn't found in the Swift");
    }
}

#[test]
fn pigeon_cores_wire_models_carry_the_operation_they_name() {
    let models = std::fs::read_to_string(pigeon_ios::root().join("PigeonCore/Models.swift"))
        .expect("Models.swift");
    assert!(
        models.matches("public let operation: String").count() >= 2,
        "ApprovalSummary and ApprovalDetail decode `operation`"
    );
}
