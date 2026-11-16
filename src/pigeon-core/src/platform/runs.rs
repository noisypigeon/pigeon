//! An instance's runs: one at a time, a queue behind it, and a bounded retry of a run whose
//! Container was lost (control-plane ADR §3). Shared by both builds; the `Instance` Durable
//! Object keeps it in its storage.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// The most attempts a run gets before it fails.
pub const MAX_ATTEMPTS: u32 = 3;

/// What a run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Plans, storing the plan.
    Plan,
    /// Applies one stored plan.
    Apply,
}

/// One run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Its ID.
    pub id: String,
    /// What it does.
    pub kind: Kind,
    /// The stored plan an apply names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// How many times it has been dispatched.
    #[serde(default)]
    pub attempts: u32,
}

/// What became of a run whose Container was lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lost {
    /// It is dispatched again, keeping its place.
    Retry(Run),
    /// It used its attempts and failed.
    Failed(Run),
}

/// Why the queue refused a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    /// A run with this ID is already queued or running.
    Duplicate(String),
    /// An apply without a plan, or a plan run naming one.
    Invalid(String),
    /// No run with this ID is running.
    NotRunning(String),
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Duplicate(id) => write!(f, "run {id} is already queued"),
            Self::Invalid(why) => f.write_str(why),
            Self::NotRunning(id) => write!(f, "run {id} is not running"),
        }
    }
}

impl std::error::Error for QueueError {}

/// One instance's runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queue {
    /// The run in progress, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<Run>,
    /// Runs waiting, oldest first.
    #[serde(default)]
    pub waiting: VecDeque<Run>,
}

impl Queue {
    /// Adds `run` behind the others: the next queue.
    ///
    /// # Errors
    ///
    /// [`QueueError::Duplicate`] for an ID already present; [`QueueError::Invalid`] for an apply
    /// without a plan or a plan run naming one. The queue is then unchanged.
    pub fn enqueue(&self, run: Run) -> Result<Self, QueueError> {
        match (run.kind, &run.plan) {
            (Kind::Apply, None) => {
                return Err(QueueError::Invalid(format!(
                    "apply {} names no plan",
                    run.id
                )));
            }
            (Kind::Plan, Some(_)) => {
                return Err(QueueError::Invalid(format!(
                    "plan run {} names a plan",
                    run.id
                )));
            }
            _ => {}
        }
        if self
            .current
            .iter()
            .chain(&self.waiting)
            .any(|r| r.id == run.id)
        {
            return Err(QueueError::Duplicate(run.id));
        }
        let mut next = self.clone();
        next.waiting.push_back(Run { attempts: 0, ..run });
        Ok(next)
    }

    /// Starts the oldest waiting run when none is running, counting the attempt: the next
    /// queue, and the run now in progress, if any.
    #[must_use]
    pub fn start(&self) -> (Self, Option<Run>) {
        let mut next = self.clone();
        if next.current.is_none() {
            next.current = next.waiting.pop_front().map(|r| Run {
                attempts: r.attempts + 1,
                ..r
            });
        }
        let current = next.current.clone();
        (next, current)
    }

    /// Ends run `id`, which must be the one in progress: the next queue, and the run.
    ///
    /// # Errors
    ///
    /// [`QueueError::NotRunning`] if `id` is not in progress; the queue is then unchanged.
    pub fn finish(&self, id: &str) -> Result<(Self, Run), QueueError> {
        match &self.current {
            Some(run) if run.id == id => {
                let mut next = self.clone();
                next.current = None;
                Ok((next, run.clone()))
            }
            _ => Err(QueueError::NotRunning(id.to_string())),
        }
    }

    /// Records that run `id`'s Container was lost: it is retried in place while it has
    /// attempts left, and fails after [`MAX_ATTEMPTS`]. The next queue, and what happened.
    ///
    /// # Errors
    ///
    /// [`QueueError::NotRunning`] if `id` is not in progress; the queue is then unchanged.
    pub fn lost(&self, id: &str) -> Result<(Self, Lost), QueueError> {
        let mut next = self.clone();
        match next.current.take() {
            Some(mut run) if run.id == id && run.attempts < MAX_ATTEMPTS => {
                run.attempts += 1;
                next.current = Some(run.clone());
                Ok((next, Lost::Retry(run)))
            }
            Some(run) if run.id == id => Ok((next, Lost::Failed(run))),
            _ => Err(QueueError::NotRunning(id.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Lost, MAX_ATTEMPTS, Queue, QueueError, Run};

    fn plan(id: &str) -> Run {
        Run {
            id: id.into(),
            kind: Kind::Plan,
            plan: None,
            attempts: 0,
        }
    }

    fn apply(id: &str, plan: &str) -> Run {
        Run {
            id: id.into(),
            kind: Kind::Apply,
            plan: Some(plan.into()),
            attempts: 0,
        }
    }

    fn id(run: Option<&Run>) -> Option<&str> {
        run.map(|r| r.id.as_str())
    }

    #[test]
    fn one_run_at_a_time_in_order() {
        let q = Queue::default().enqueue(plan("a")).expect("a");
        let q = q.enqueue(apply("b", "p1")).expect("b");
        let (q, running) = q.start();
        assert_eq!(running.map(|r| (r.id, r.attempts)), Some(("a".into(), 1)));
        let (q, running) = q.start();
        assert_eq!(id(running.as_ref()), Some("a"), "a still runs");
        assert_eq!(q.finish("b"), Err(QueueError::NotRunning("b".into())));
        let (q, done) = q.finish("a").expect("a");
        assert_eq!(done.id, "a");
        let (q, running) = q.start();
        assert_eq!(id(running.as_ref()), Some("b"));
        let (q, _) = q.finish("b").expect("b");
        assert!(q.start().1.is_none());
    }

    #[test]
    fn a_lost_run_retries_in_place_then_fails() {
        let q = Queue::default().enqueue(apply("a", "p1")).expect("a");
        let (mut q, _) = q.enqueue(plan("b")).expect("b").start();
        for attempt in 2..=MAX_ATTEMPTS {
            let (lost, what) = q.lost("a").expect("lost");
            match what {
                Lost::Retry(r) => assert_eq!(r.attempts, attempt),
                Lost::Failed(r) => panic!("failed early: {r:?}"),
            }
            let (started, running) = lost.start();
            assert_eq!(id(running.as_ref()), Some("a"), "it keeps its place");
            q = started;
        }
        let (q, what) = q.lost("a").expect("lost");
        assert!(matches!(what, Lost::Failed(r) if r.attempts == MAX_ATTEMPTS));
        let (q, running) = q.start();
        assert_eq!(id(running.as_ref()), Some("b"), "the next run starts");
        assert_eq!(q.lost("a"), Err(QueueError::NotRunning("a".into())));
    }

    #[test]
    fn bad_and_duplicate_runs_are_refused() {
        let q = Queue::default();
        let no_plan = Run {
            plan: None,
            ..apply("a", "p")
        };
        assert!(matches!(q.enqueue(no_plan), Err(QueueError::Invalid(_))));
        let plan_with_one = Run {
            plan: Some("p".into()),
            ..plan("b")
        };
        assert!(matches!(
            q.enqueue(plan_with_one),
            Err(QueueError::Invalid(_))
        ));
        let (q, _) = q.enqueue(plan("c")).expect("c").start();
        assert_eq!(
            q.enqueue(plan("c")),
            Err(QueueError::Duplicate("c".into())),
            "running"
        );
        let q = q.enqueue(plan("d")).expect("d");
        assert_eq!(
            q.enqueue(plan("d")),
            Err(QueueError::Duplicate("d".into())),
            "waiting"
        );
    }

    #[test]
    fn the_queue_round_trips_its_stored_form() {
        let q = Queue::default().enqueue(apply("a", "p1")).expect("a");
        let (q, _) = q.enqueue(plan("b")).expect("b").start();
        let text = serde_json::to_string(&q).expect("serialize");
        assert_eq!(
            serde_json::from_str::<Queue>(&text).expect("deserialize"),
            q
        );
    }
}
