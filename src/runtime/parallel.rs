//! Bounded parallel preparation with canonical source-order delivery.

use super::{MetricEvent, MetricPhase, MetricValues, MetricsHook};
use crate::core::OperationKind;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::thread;

#[cfg(feature = "schemas")]
use schemars::JsonSchema;

/// Supported independent source-unit families.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ParallelUnitKind {
    Page,
    Sheet,
    Slide,
    ArchiveMember,
    RepositoryFile,
    Generic,
}

/// Numeric source order, independent of completion timing or sensitive labels.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CanonicalOrderKey {
    pub primary: u64,
    pub secondary: u64,
}

impl CanonicalOrderKey {
    pub const fn new(primary: u64, secondary: u64) -> Self {
        Self { primary, secondary }
    }

    pub const fn source_order(order: u64) -> Self {
        Self::new(order, 0)
    }
}

/// Explicit concurrency and memory-window policy.
#[cfg_attr(feature = "schemas", derive(JsonSchema))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ParallelismOptions {
    pub workers: usize,
    pub max_in_flight: usize,
}

impl ParallelismOptions {
    pub const fn serial() -> Self {
        Self {
            workers: 1,
            max_in_flight: 1,
        }
    }

    pub const fn new(workers: usize, max_in_flight: usize) -> Self {
        Self {
            workers,
            max_in_flight,
        }
    }

    pub fn validate(self) -> Result<(), ParallelError<std::convert::Infallible>> {
        if self.workers == 0 {
            return Err(ParallelError::ZeroWorkers);
        }
        if self.max_in_flight == 0 {
            return Err(ParallelError::ZeroInFlight);
        }
        Ok(())
    }
}

impl Default for ParallelismOptions {
    fn default() -> Self {
        Self::serial()
    }
}

#[derive(Debug)]
pub struct ParallelJob<T> {
    pub order: CanonicalOrderKey,
    pub kind: ParallelUnitKind,
    pub input: T,
}

impl<T> ParallelJob<T> {
    pub const fn new(order: CanonicalOrderKey, kind: ParallelUnitKind, input: T) -> Self {
        Self { order, kind, input }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParallelOutput<T> {
    pub order: CanonicalOrderKey,
    pub kind: ParallelUnitKind,
    pub output: T,
}

#[derive(Debug, thiserror::Error)]
pub enum ParallelError<E>
where
    E: Error + 'static,
{
    #[error("parallelism workers must be greater than zero")]
    ZeroWorkers,
    #[error("parallelism max_in_flight must be greater than zero")]
    ZeroInFlight,
    #[error("parallel unit order keys must be unique: {0:?}")]
    DuplicateOrder(CanonicalOrderKey),
    #[error("bounded streaming input is not in canonical order: {previous:?} then {next:?}")]
    OutOfOrder {
        previous: CanonicalOrderKey,
        next: CanonicalOrderKey,
    },
    #[error("parallel worker panicked")]
    WorkerPanic,
    #[error("parallel unit {order:?} failed: {source}")]
    Task {
        order: CanonicalOrderKey,
        #[source]
        source: E,
    },
}

pub fn deterministic_parallel_collect<I, T, R, E, F>(
    jobs: I,
    options: ParallelismOptions,
    execute: F,
) -> Result<Vec<ParallelOutput<R>>, ParallelError<E>>
where
    I: IntoIterator<Item = ParallelJob<T>>,
    T: Send,
    R: Send,
    E: Error + Send + 'static,
    F: Fn(T) -> Result<R, E> + Sync,
{
    deterministic_parallel_collect_with_metrics(
        jobs,
        options,
        OperationKind::Parse,
        &MetricsHook::default(),
        execute,
    )
}

pub fn deterministic_parallel_collect_with_metrics<I, T, R, E, F>(
    jobs: I,
    options: ParallelismOptions,
    operation: OperationKind,
    metrics: &MetricsHook,
    execute: F,
) -> Result<Vec<ParallelOutput<R>>, ParallelError<E>>
where
    I: IntoIterator<Item = ParallelJob<T>>,
    T: Send,
    R: Send,
    E: Error + Send + 'static,
    F: Fn(T) -> Result<R, E> + Sync,
{
    validate_options(options)?;
    let mut jobs = jobs.into_iter().collect::<Vec<_>>();
    jobs.sort_by_key(|job| job.order);
    reject_duplicates(&jobs)?;
    let mut output = Vec::with_capacity(jobs.len());
    for chunk in chunk_jobs(jobs, options.max_in_flight) {
        output.extend(run_chunk(chunk, options.workers, &execute)?);
    }
    emit_parallel_metrics(metrics, operation, output.len(), options);
    Ok(output)
}

pub fn deterministic_parallel_for_each_ordered<I, T, R, E, F, S>(
    jobs: I,
    options: ParallelismOptions,
    execute: F,
    emit: S,
) -> Result<(), ParallelError<E>>
where
    I: IntoIterator<Item = ParallelJob<T>>,
    T: Send,
    R: Send,
    E: Error + Send + 'static,
    F: Fn(T) -> Result<R, E> + Sync,
    S: FnMut(ParallelOutput<R>),
{
    deterministic_parallel_for_each_ordered_with_metrics(
        jobs,
        options,
        OperationKind::Parse,
        &MetricsHook::default(),
        execute,
        emit,
    )
}

pub fn deterministic_parallel_for_each_ordered_with_metrics<I, T, R, E, F, S>(
    jobs: I,
    options: ParallelismOptions,
    operation: OperationKind,
    metrics: &MetricsHook,
    execute: F,
    mut emit: S,
) -> Result<(), ParallelError<E>>
where
    I: IntoIterator<Item = ParallelJob<T>>,
    T: Send,
    R: Send,
    E: Error + Send + 'static,
    F: Fn(T) -> Result<R, E> + Sync,
    S: FnMut(ParallelOutput<R>),
{
    validate_options(options)?;
    let mut input = jobs.into_iter();
    let mut previous = None;
    let mut completed = 0_usize;
    loop {
        let mut chunk = Vec::with_capacity(options.max_in_flight);
        for _ in 0..options.max_in_flight {
            let Some(job) = input.next() else {
                break;
            };
            if let Some(last) = previous {
                if job.order == last {
                    return Err(ParallelError::DuplicateOrder(job.order));
                }
                if job.order < last {
                    return Err(ParallelError::OutOfOrder {
                        previous: last,
                        next: job.order,
                    });
                }
            }
            previous = Some(job.order);
            chunk.push(job);
        }
        if chunk.is_empty() {
            break;
        }
        let output = run_chunk(chunk, options.workers, &execute)?;
        completed = completed.saturating_add(output.len());
        output.into_iter().for_each(&mut emit);
    }
    emit_parallel_metrics(metrics, operation, completed, options);
    Ok(())
}

fn validate_options<E: Error + 'static>(
    options: ParallelismOptions,
) -> Result<(), ParallelError<E>> {
    if options.workers == 0 {
        return Err(ParallelError::ZeroWorkers);
    }
    if options.max_in_flight == 0 {
        return Err(ParallelError::ZeroInFlight);
    }
    Ok(())
}

fn reject_duplicates<T, E: Error + 'static>(
    jobs: &[ParallelJob<T>],
) -> Result<(), ParallelError<E>> {
    if let Some(pair) = jobs.windows(2).find(|pair| pair[0].order == pair[1].order) {
        return Err(ParallelError::DuplicateOrder(pair[0].order));
    }
    Ok(())
}

fn chunk_jobs<T>(
    jobs: Vec<ParallelJob<T>>,
    size: usize,
) -> impl Iterator<Item = Vec<ParallelJob<T>>> {
    let mut jobs = jobs.into_iter();
    std::iter::from_fn(move || {
        let chunk = jobs.by_ref().take(size).collect::<Vec<_>>();
        (!chunk.is_empty()).then_some(chunk)
    })
}

fn run_chunk<T, R, E, F>(
    jobs: Vec<ParallelJob<T>>,
    workers: usize,
    execute: &F,
) -> Result<Vec<ParallelOutput<R>>, ParallelError<E>>
where
    T: Send,
    R: Send,
    E: Error + Send + 'static,
    F: Fn(T) -> Result<R, E> + Sync,
{
    let worker_count = workers.min(jobs.len()).max(1);
    let mut buckets = std::iter::repeat_with(Vec::new)
        .take(worker_count)
        .collect::<Vec<Vec<ParallelJob<T>>>>();
    for (index, job) in jobs.into_iter().enumerate() {
        buckets[index % worker_count].push(job);
    }
    let joined = thread::scope(|scope| {
        let handles = buckets
            .into_iter()
            .map(|bucket| {
                scope.spawn(move || {
                    bucket
                        .into_iter()
                        .map(|job| (job.order, job.kind, execute(job.input)))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join())
            .collect::<Result<Vec<_>, _>>()
    })
    .map_err(|_| ParallelError::WorkerPanic)?;
    let mut joined = joined.into_iter().flatten().collect::<Vec<_>>();
    joined.sort_by_key(|(order, _, _)| *order);
    joined
        .into_iter()
        .map(|(order, kind, result)| {
            result
                .map(|output| ParallelOutput {
                    order,
                    kind,
                    output,
                })
                .map_err(|source| ParallelError::Task { order, source })
        })
        .collect()
}

fn emit_parallel_metrics(
    metrics: &MetricsHook,
    operation: OperationKind,
    completed: usize,
    options: ParallelismOptions,
) {
    metrics.emit(&MetricEvent::new(
        operation,
        MetricPhase::Parallelism,
        MetricValues {
            completed_units: u64::try_from(completed).unwrap_or(u64::MAX),
            peak_in_flight: u64::try_from(options.max_in_flight.min(completed)).unwrap_or(u64::MAX),
            ..MetricValues::default()
        },
    ));
}
