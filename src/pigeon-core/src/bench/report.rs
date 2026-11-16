//! A bench run's outcome, as the CLI reports it.

use super::deal::Check;

/// The process exit code for a run's final `state` and `exit`.
#[must_use]
pub fn exit_code(state: &str, exit: Option<i64>) -> i32 {
    match state {
        "passed" => 0,
        "failed" => exit.map_or(1, |e| i32::try_from(e.clamp(1, 255)).unwrap_or(1)),
        "timed_out" => 124,
        "cancelled" => 130,
        _ => 1,
    }
}

/// The checks in scope's JSON answer, found in a log among other output.
///
/// # Errors
/// When the log holds no array of checks.
pub fn checks_in(log: &str) -> Result<Vec<Check>, String> {
    let err = || "bench: no checks in scope's answer".to_string();
    let lines: Vec<&str> = log.lines().collect();
    let first = lines
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .ok_or_else(err)?;
    let last = if lines[first].trim_end().ends_with(']') {
        first
    } else {
        lines
            .iter()
            .rposition(|l| l.trim_start().starts_with(']'))
            .ok_or_else(err)?
    };
    if last < first {
        return Err(err());
    }
    serde_json::from_str(&lines[first..=last].join("\n")).map_err(|_| err())
}

/// True when a refusal says to wait and retry.
#[must_use]
pub fn waiting(why: &str) -> bool {
    why.starts_with("wait:")
}

/// True when a run has left `syncing` and `running`.
#[must_use]
pub fn done(state: &str) -> bool {
    !matches!(state, "syncing" | "running")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes() {
        assert_eq!(exit_code("passed", Some(0)), 0);
        assert_eq!(exit_code("failed", Some(3)), 3);
        assert_eq!(exit_code("failed", Some(0)), 1);
        assert_eq!(exit_code("failed", Some(300)), 255);
        assert_eq!(exit_code("failed", None), 1);
        assert_eq!(exit_code("timed_out", None), 124);
        assert_eq!(exit_code("cancelled", None), 130);
        assert_eq!(exit_code("weird", None), 1);
    }

    #[test]
    fn checks_found_in_noise() {
        let log = "Compiling x\n[\n  {\"name\": \"fmt\", \"argv\": [\"a\"]},\n  {\"name\": \"t\", \"argv\": [\"b\", \"c\"]}\n]\nFinished\n";
        let c = checks_in(log).unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[1].name, "t");
        assert_eq!(c[1].argv, ["b", "c"]);
    }

    #[test]
    fn checks_edge_cases() {
        assert_eq!(checks_in("noise\n[]\nmore").unwrap().len(), 0);
        assert!(checks_in("nothing here").is_err());
        assert!(checks_in("[\n{broken\n]").is_err());
    }

    #[test]
    fn wait_and_done() {
        assert!(waiting("wait: capacity"));
        assert!(!waiting("refused"));
        assert!(!done("syncing"));
        assert!(!done("running"));
        assert!(done("passed"));
    }
}
