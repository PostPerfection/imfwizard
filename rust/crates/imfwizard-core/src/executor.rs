//! In-memory job executor.
//!
//! Runs queued jobs through the same core code paths the CLI uses. The queue is
//! process-local (postkit's `JobQueue` is an in-memory `Arc<Mutex<..>>`): jobs
//! live only for the lifetime of the process that owns the queue. There is no
//! persistence or cross-process IPC, so this is only useful behind a long-lived
//! process such as the REST server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::job_queue::{Job, JobQueue, JobState, JobType};

#[derive(Clone, Default)]
pub struct ExecutorQueue {
    queue: JobQueue,
    // every state change holds this lock: postkit's set_state overwrites a cancel
    running_cancel_flags: Arc<Mutex<HashMap<u64, Arc<AtomicBool>>>>,
}

impl ExecutorQueue {
    pub fn submit(&self, job: Job) -> u64 {
        self.queue.submit(job)
    }

    pub fn get(&self, id: u64) -> Option<Job> {
        self.queue.get(id)
    }

    pub fn list(&self) -> Vec<Job> {
        self.queue.list()
    }

    pub fn cancel(&self, id: u64) -> bool {
        let flags = self.running_cancel_flags.lock().unwrap();
        if !self.queue.cancel(id) {
            return false;
        }
        if let Some(flag) = flags.get(&id) {
            flag.store(true, Ordering::Relaxed);
        }
        true
    }

    fn start(&self, id: u64) -> Option<Arc<AtomicBool>> {
        let mut flags = self.running_cancel_flags.lock().unwrap();
        if !self.is_in_state(id, JobState::Queued) {
            return None;
        }
        self.queue.set_state(id, JobState::Running);
        let cancel = Arc::new(AtomicBool::new(false));
        flags.insert(id, cancel.clone());
        Some(cancel)
    }

    fn finish(&self, id: u64, outcome: Result<(), String>) {
        let mut flags = self.running_cancel_flags.lock().unwrap();
        flags.remove(&id);
        if !self.is_in_state(id, JobState::Running) {
            return;
        }
        match outcome {
            Ok(()) => {
                self.queue.set_progress(id, 1.0);
                self.queue.set_state(id, JobState::Completed);
            }
            Err(e) => self.queue.fail(id, &e),
        }
    }

    fn is_in_state(&self, id: u64, state: JobState) -> bool {
        self.queue.get(id).is_some_and(|job| job.state == state)
    }
}

/// Run a single job to completion, mapping its type onto a real core operation.
///
/// `input`/`output`/`description` are the only parameters the queue carries, so
/// richer jobs (Create) take their title from `description`. Job types that need
/// parameters the queue cannot express fail loud rather than silently no-op.
pub fn execute_job(job: &Job, cancel: &Arc<AtomicBool>) -> Result<(), String> {
    match job.job_type {
        JobType::Encode => crate::encode::encode_image_sequence(
            &job.input,
            &job.output,
            crate::encode::DEFAULT_ENCODE_BITRATE_MBPS,
            crate::encode::FrameRate::default(),
            cancel,
        )
        .map(|_| ()),
        JobType::Transcode => {
            let opts = crate::transcode::TranscodeOptions {
                input: job.input.clone(),
                output: job.output.clone(),
                cancel: cancel.clone(),
                ..Default::default()
            };
            let r = crate::transcode::transcode(&opts);
            if r.success { Ok(()) } else { Err(r.error) }
        }
        JobType::Validate => {
            let r = crate::validate::validate_imp(&job.input);
            if r.valid {
                Ok(())
            } else {
                Err(r.errors.join("; "))
            }
        }
        JobType::Loudness => {
            if !job.input.exists() {
                return Err(format!("input not found: {}", job.input.display()));
            }
            let _ = postkit::loudness::measure_loudness(&job.input);
            Ok(())
        }
        JobType::Create => {
            let opts = crate::imp::ImpOptions {
                output_dir: job.output.clone(),
                compositions: vec![crate::imp::Composition {
                    title: job.description.clone(),
                    content_kind: "feature".to_string(),
                    j2k_dir: Some(job.input.clone()),
                    ..Default::default()
                }],
                fps_num: 24,
                fps_den: 1,
                cancel: cancel.clone(),
                ..Default::default()
            };
            let r = crate::imp::create_imp(&opts);
            if r.success { Ok(()) } else { Err(r.error) }
        }
        // These need parameters the queue cannot carry; fail loud instead of
        // pretending to run them.
        JobType::Qc | JobType::Copy => Err(format!(
            "job type {:?} is not runnable via the queue; use the dedicated CLI command",
            job.job_type
        )),
        // imfwizard encrypts no IMP, so it writes no KDM at all
        JobType::Kdm => Err("job type Kdm is not runnable: imfwizard writes no KDM".to_string()),
    }
}

/// Worker loop: pick the next runnable job, run it, record the outcome. Runs
/// until `stop` is set and no runnable job remains. One worker per queue.
pub fn run_worker(queue: ExecutorQueue, stop: Arc<AtomicBool>) {
    loop {
        match queue.queue.next_runnable() {
            Some(job) => {
                if let Some(cancel) = queue.start(job.id) {
                    queue.finish(job.id, execute_job(&job, &cancel));
                }
            }
            None => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Spawn `run_worker` on a background thread over a clone of the queue. The
/// returned flag stops the worker when set (after the current job, if any).
pub fn spawn_worker(queue: &ExecutorQueue) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let queue = queue.clone();
    let stop_clone = stop.clone();
    std::thread::spawn(move || run_worker(queue, stop_clone));
    stop
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn unsupported_job_types_fail_loud() {
        let job = Job {
            job_type: JobType::Kdm,
            ..Default::default()
        };
        assert!(execute_job(&job, &Arc::default()).is_err());
    }

    #[test]
    fn validate_job_runs_and_reports() {
        // A non-directory input must fail the validate job, proving the executor
        // actually invokes validation rather than dropping the job.
        let job = Job {
            job_type: JobType::Validate,
            input: PathBuf::from("/nonexistent/imp/dir"),
            ..Default::default()
        };
        assert!(execute_job(&job, &Arc::default()).is_err());
    }

    #[test]
    fn worker_drains_queue_and_records_state() {
        let queue = ExecutorQueue::default();
        let id = queue.submit(Job {
            job_type: JobType::Validate,
            input: PathBuf::from("/nonexistent/imp/dir"),
            ..Default::default()
        });
        let stop = Arc::new(AtomicBool::new(true));
        run_worker(queue.clone(), stop);
        // the job must have been executed (and failed), not left queued
        assert_eq!(queue.get(id).unwrap().state, JobState::Failed);
    }
}
