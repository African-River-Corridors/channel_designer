//! Running one stage off the main thread, with a Cancel that stops it (spec 0003, ticket 70).
//!
//! The engine has no cancel hook, but every stage that takes time reads the DEM through a terrain
//! sampler. The job wraps that sampler: once Cancel is set, the next read stops the stage by
//! unwinding out of it, and the job's copy of the design is dropped. The design the app holds is
//! only replaced when a stage finishes, so a cancelled or failed run leaves it unchanged.
//!
//! No Tauri types here, so it is tested like any other Rust.

use cd_format::design::terrain::dem_sampler;
use cd_format::design::{AlignMethod, Session, Stage, Terrain};
use cd_format::geotiff::Dem;
use serde_json::Value;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The marker a cancelled read unwinds with.
struct Cancelled;

/// What the user and the job share.
#[derive(Debug, Default)]
pub struct RunControl {
    pub cancel: AtomicBool,
    /// How many terrain reads (points) the stage has asked for: the only progress the engine gives.
    pub samples: AtomicU64,
}

/// What a stage run ended as.
#[derive(Debug)]
pub enum Outcome {
    Done(Box<Session>),
    Cancelled,
    Failed(String),
}

/// One stage to run on a copy of the design.
#[derive(Debug)]
pub struct RunJob {
    pub session: Session,
    pub stage: Stage,
    /// For alignment: which method to add or rerun. `None` reruns the methods run before.
    pub method: Option<AlignMethod>,
    /// The DEM, when it is the file whose hash the design recorded.
    pub dem: Option<Arc<Dem>>,
    pub project_epsg: Option<u32>,
    pub control: Arc<RunControl>,
}

impl RunJob {
    /// Run the stage. `progress` is called with the number of reads so far, from the worker.
    pub fn execute(self, progress: &(dyn Fn(u64) + Sync)) -> Outcome {
        let RunJob {
            mut session,
            stage,
            method,
            dem,
            project_epsg,
            control,
        } = self;
        let base = match dem.as_deref().map(|d| dem_sampler(d, project_epsg)) {
            Some(Ok(s)) => Some(s),
            Some(Err(e)) => return Outcome::Failed(e.to_string()),
            None => None,
        };
        let wrapped = |xs: &[f64], ys: &[f64]| -> Vec<f64> {
            if control.cancel.load(Ordering::Relaxed) {
                resume_unwind(Box::new(Cancelled));
            }
            let n = control
                .samples
                .fetch_add(xs.len() as u64, Ordering::Relaxed)
                + xs.len() as u64;
            progress(n);
            (base
                .as_ref()
                .expect("a sampler is only called when there is a DEM"))(xs, ys)
        };
        let terrain: Option<Terrain> = base.is_some().then_some(&wrapped as Terrain);
        let ran = catch_unwind(AssertUnwindSafe(|| match (stage, method) {
            (Stage::Alignment, Some(m)) => session.align(m, &Value::Null, terrain),
            _ => session.run_stage_with(stage, terrain),
        }));
        match ran {
            Ok(Ok(())) => Outcome::Done(Box::new(session)),
            Ok(Err(e)) => Outcome::Failed(e.to_string()),
            Err(payload) if payload.is::<Cancelled>() => Outcome::Cancelled,
            Err(_) => Outcome::Failed("the stage stopped unexpectedly".into()),
        }
    }
}
