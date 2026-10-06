//! Job executor for the REST server.
//!
//! Runs queued jobs through the same core code paths the CLI uses. The queue is
//! postkit's `JobQueue`, which writes every job to a jobs file, so queued jobs
//! and finished ones survive a server restart.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use postkit::job_queue::{JobQueue, JobState, QueueJob};
use serde::{Deserialize, Serialize};

const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum JobType {
    Create,
    Validate,
    Encode,
    Transcode,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RestJob {
    pub id: u64,
    pub job_type: JobType,
    pub title: String,
    pub input: PathBuf,
    pub output: PathBuf,
}

impl QueueJob for RestJob {
    fn id(&self) -> u64 {
        self.id
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn output_dir(&self) -> Option<&Path> {
        if self.output.as_os_str().is_empty() {
            return None;
        }
        Some(&self.output)
    }
}

pub type RestJobQueue = JobQueue<RestJob>;

/// Run a single job to completion, mapping its type onto a real core operation.
///
/// `input`/`output`/`title` are the only parameters the queue carries, so
/// richer jobs (Create) take their ContentTitle from `title`.
pub fn execute_job(
    job: &RestJob,
    encode_threads: u32,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    match job.job_type {
        JobType::Encode => crate::encode::encode_image_sequence(
            &job.input,
            &job.output,
            crate::encode::DEFAULT_ENCODE_BITRATE_MBPS,
            crate::encode::FrameRate::default(),
            encode_threads,
            cancel,
        )
        .map(|_| ()),
        JobType::Transcode => {
            let opts = postkit::transcode::TranscodeOptions {
                input: job.input.clone(),
                output: job.output.clone(),
                cancel: cancel.clone(),
                ..Default::default()
            };
            let r = postkit::transcode::transcode(&opts);
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
        JobType::Create => {
            let opts = crate::imp::ImpOptions {
                output_dir: job.output.clone(),
                compositions: vec![crate::imp::Composition {
                    title: job.title.clone(),
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
    }
}

// a cancelled job may still return Ok or a stop error
fn end_state(cancelled: bool, outcome: Result<(), String>) -> (JobState, String) {
    if cancelled {
        return (JobState::Cancelled, String::new());
    }
    match outcome {
        Ok(()) => (JobState::Completed, String::new()),
        Err(error) => (JobState::Failed, error),
    }
}

/// Worker loop: take the next queued job, run it, record the outcome. Runs
/// until `stop` is set and no queued job remains. One worker per queue.
pub fn run_worker(queue: Arc<RestJobQueue>, encode_threads: u32, stop: Arc<AtomicBool>) {
    loop {
        match queue.take_next() {
            Some(job) => {
                queue.start(&job);
                let cancel = queue.cancel_flag();
                let outcome = execute_job(&job, encode_threads, &cancel);
                let (state, message) = end_state(cancel.load(Ordering::Relaxed), outcome);
                queue.finish(&job, state, &message);
                queue.clear_current();
            }
            None => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(IDLE_POLL_INTERVAL);
            }
        }
    }
}

/// Spawn `run_worker` on a background thread over the shared queue. The
/// returned flag stops the worker when set (after the current job, if any).
pub fn spawn_worker(queue: &Arc<RestJobQueue>, encode_threads: u32) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let queue = queue.clone();
    let stop_clone = stop.clone();
    std::thread::spawn(move || run_worker(queue, encode_threads, stop_clone));
    stop
}

#[cfg(test)]
mod tests {
    use super::*;

    const MISSING_IMP_DIRECTORY: &str = "/nonexistent/imp/dir";

    fn validate_job(id: u64) -> RestJob {
        RestJob {
            id,
            job_type: JobType::Validate,
            title: "validate".into(),
            input: PathBuf::from(MISSING_IMP_DIRECTORY),
            output: PathBuf::new(),
        }
    }

    fn queue_in(directory: &tempfile::TempDir) -> Arc<RestJobQueue> {
        Arc::new(RestJobQueue::new(directory.path().join("rest-jobs.jsonl")))
    }

    fn drain(queue: &Arc<RestJobQueue>) {
        let stop = Arc::new(AtomicBool::new(true));
        run_worker(
            queue.clone(),
            crate::preferences::AUTOMATIC_ENCODE_THREADS,
            stop,
        );
    }

    #[test]
    fn validate_job_runs_and_reports() {
        // A non-directory input must fail the validate job, proving the executor
        // actually invokes validation rather than dropping the job.
        let error = execute_job(
            &validate_job(1),
            crate::preferences::AUTOMATIC_ENCODE_THREADS,
            &Arc::default(),
        )
        .unwrap_err();
        assert!(error.contains(MISSING_IMP_DIRECTORY), "{error}");
    }

    #[test]
    fn worker_drains_queue_and_records_state() {
        let directory = tempfile::tempdir().unwrap();
        let queue = queue_in(&directory);
        let id = queue.reserve_job_id();
        queue.submit(validate_job(id));
        drain(&queue);
        // the job must have been executed (and failed), not left queued
        let job = queue.get(id).unwrap();
        assert_eq!(job.state, JobState::Failed);
        assert!(
            job.message.contains(MISSING_IMP_DIRECTORY),
            "{}",
            job.message
        );
    }

    #[test]
    fn a_job_cancelled_while_queued_never_runs() {
        let directory = tempfile::tempdir().unwrap();
        let queue = queue_in(&directory);
        let id = queue.reserve_job_id();
        queue.submit(validate_job(id));
        assert!(queue.cancel(id));
        drain(&queue);
        // a run would have failed it on the missing directory
        let job = queue.get(id).unwrap();
        assert_eq!(job.state, JobState::Cancelled);
        assert_eq!(job.message, "");
    }
}
