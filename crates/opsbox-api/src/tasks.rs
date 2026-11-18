//! Long operations (docs/BACKEND_SPEC.md 7.1): started in the background, polled by id.
//! Only one task of a kind runs at a time; a second request is a conflict that names the
//! running one. Finished tasks are kept for a day.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use opsbox_proto::ErrorCode;
use serde::Serialize;
use serde_json::Value;

use crate::HttpError;

const KEEP_SECS: i64 = 24 * 3600;
const MAX_TASKS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Running,
    Ok,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub kind: String,
    pub status: Status,
    /// The last lines of output, when the operation produced any.
    pub log: Vec<String>,
    /// What went wrong, in words for a person.
    pub error: Option<String>,
    /// The operation's own answer (for example how many packages were installed).
    pub result: Option<Value>,
    pub started: i64,
    pub finished: Option<i64>,
}

#[derive(Clone, Default)]
pub struct Tasks {
    inner: Arc<Mutex<HashMap<String, Task>>>,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// What a finished operation reports.
pub struct Outcome {
    pub log: Vec<String>,
    pub result: Value,
}

impl Tasks {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Task>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Start `work` unless a task of `kind` is already running. Returns the task id.
    pub fn start<F>(&self, kind: &str, work: F) -> Result<String, HttpError>
    where
        F: Future<Output = Result<Outcome, HttpError>> + Send + 'static,
    {
        let id = ulid::Ulid::generate().to_string();
        {
            let mut tasks = self.lock();
            let t = now();
            tasks.retain(|_, v| v.finished.is_none_or(|f| t - f < KEEP_SECS));
            if let Some(running) = tasks
                .values()
                .find(|v| v.kind == kind && v.status == Status::Running)
            {
                return Err(HttpError::new(
                    ErrorCode::Conflict,
                    format!(
                        "This is already running (task {}). Wait for it to finish.",
                        running.id
                    ),
                ));
            }
            if tasks.len() >= MAX_TASKS {
                if let Some(oldest) = tasks
                    .values()
                    .filter(|v| v.finished.is_some())
                    .min_by_key(|v| (v.started, v.id.clone()))
                    .map(|v| v.id.clone())
                {
                    tasks.remove(&oldest);
                } else {
                    return Err(HttpError::new(
                        ErrorCode::RateLimited,
                        "Too many operations are running. Wait for one to finish.",
                    ));
                }
            }
            tasks.insert(
                id.clone(),
                Task {
                    id: id.clone(),
                    kind: kind.to_owned(),
                    status: Status::Running,
                    log: vec![],
                    error: None,
                    result: None,
                    started: t,
                    finished: None,
                },
            );
        }
        let me = self.clone();
        let tid = id.clone();
        tokio::spawn(async move {
            let outcome = work.await;
            let mut tasks = me.lock();
            if let Some(task) = tasks.get_mut(&tid) {
                task.finished = Some(now());
                match outcome {
                    Ok(o) => {
                        task.status = Status::Ok;
                        task.log = o
                            .log
                            .into_iter()
                            .take(100)
                            .map(|l| l.chars().take(8192).collect())
                            .collect();
                        task.result = Some(o.result);
                    }
                    Err(e) => {
                        task.status = Status::Failed;
                        task.error = Some(e.0.error.message);
                    }
                }
            }
        });
        Ok(id)
    }

    pub(crate) fn is_running(&self, kind: &str) -> bool {
        self.lock()
            .values()
            .any(|t| t.kind == kind && t.status == Status::Running)
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.lock().get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    async fn finished(t: &Tasks, id: &str) -> Task {
        for _ in 0..100 {
            let task = t.get(id).unwrap();
            if task.status != Status::Running {
                return task;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("task did not finish");
    }

    #[tokio::test]
    async fn a_task_runs_in_the_background_and_reports_its_result() {
        let t = Tasks::default();
        let id = t
            .start("x", async {
                Ok(Outcome {
                    log: vec!["done".into()],
                    result: serde_json::json!({"n": 3}),
                })
            })
            .unwrap();
        let task = finished(&t, &id).await;
        assert_eq!(
            (task.status, task.log, task.result),
            (
                Status::Ok,
                vec!["done".to_owned()],
                Some(serde_json::json!({"n": 3}))
            )
        );
        assert!(task.finished.is_some() && task.error.is_none());
    }

    #[tokio::test]
    async fn a_failure_keeps_its_sentence() {
        let t = Tasks::default();
        let id = t
            .start("x", async {
                Err(HttpError::new(
                    ErrorCode::Internal,
                    "Could not refresh the package lists.",
                ))
            })
            .unwrap();
        let task = finished(&t, &id).await;
        assert_eq!(
            (task.status, task.error.as_deref()),
            (Status::Failed, Some("Could not refresh the package lists."))
        );
    }

    #[tokio::test]
    async fn only_one_task_of_a_kind_at_a_time() {
        let t = Tasks::default();
        let (release, gate) = tokio::sync::oneshot::channel::<()>();
        let first = t
            .start("updates", async move {
                let _ = gate.await;
                Ok(Outcome {
                    log: vec![],
                    result: Value::Null,
                })
            })
            .unwrap();
        let second = t.start("updates", async {
            Ok(Outcome {
                log: vec![],
                result: Value::Null,
            })
        });
        let e = second.expect_err("conflict");
        assert_eq!(e.0.error.code, ErrorCode::Conflict);
        assert!(
            e.0.error.message.contains(&first),
            "names the running task: {}",
            e.0.error.message
        );
        // A different kind is fine, and the kind is free again once the first finishes.
        t.start("swap", async {
            Ok(Outcome {
                log: vec![],
                result: Value::Null,
            })
        })
        .unwrap();
        release.send(()).unwrap();
        finished(&t, &first).await;
        t.start("updates", async {
            Ok(Outcome {
                log: vec![],
                result: Value::Null,
            })
        })
        .unwrap();
    }

    #[test]
    fn unknown_ids_are_none() {
        assert!(Tasks::default().get("nope").is_none());
    }
}
