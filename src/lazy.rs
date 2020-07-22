use crate::cache::*;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::{
    any::{Any, TypeId},
    collections::HashSet,
    future::Future,
    hash::{Hash, Hasher},
    marker::PhantomData,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock, Weak,
    },
};

pub use turbosloth_macros::IntoLazy;

pub trait LazyReqs: Any + Sized + Send + Sync + 'static {}
impl<T: Any + Sized + Send + Sync + 'static> LazyReqs for T {}

#[async_trait]
pub trait LazyWorker: Send + Sync + 'static {
    type Output: LazyReqs;

    async fn run(self, context: RunContext) -> Result<Self::Output>;
}

type BoxedWorkerFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn Any + Send + Sync>>> + Send + 'static>>;

pub trait LazyWorkerObj: Send + Sync {
    fn identity(&self) -> u64;
    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj>;
    fn run_boxed(self: Box<Self>, context: RunContext) -> BoxedWorkerFuture;
    fn debug_name(&self) -> &'static str;
}

impl<T: LazyReqs, W> LazyWorkerObj for W
where
    W: LazyWorker<Output = T> + Clone + Hash,
{
    fn identity(&self) -> u64 {
        <Self as ToLazyIdentity>::identity(self)
    }

    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj> {
        Box::new((*self).clone())
    }

    fn run_boxed(self: Box<Self>, context: RunContext) -> BoxedWorkerFuture {
        Box::pin(async {
            (*self)
                .run(context)
                .await
                .map(|result| -> Arc<dyn Any + Send + Sync> { Arc::new(result) })
        })
    }

    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

pub struct LazyPayload {
    pub worker: Box<dyn LazyWorkerObj>,
    pub build_record: RwLock<BuildRecord>,
    pub rebuild_pending: AtomicBool,
}

impl LazyPayload {
    pub(crate) fn set_new_build_result(
        &self,
        artifact: Option<Arc<dyn Any + Send + Sync>>,
        dependencies: HashSet<BuildDependency>,
    ) -> BuildRecordDiff {
        let mut build_record = self.build_record.write().unwrap();

        let prev_deps = std::mem::take(&mut build_record.dependencies);

        let added_deps = dependencies.difference(&prev_deps).cloned().collect();
        let removed_deps = prev_deps.difference(&dependencies).cloned().collect();

        build_record.dependencies = dependencies;
        build_record.artifact = artifact;

        // Filter out invalid reverse dependencies
        build_record
            .reverse_dependencies
            .retain(|rev| rev.upgrade().is_some());

        BuildRecordDiff {
            added_deps,
            removed_deps,
        }
    }

    fn invalidate(&self) {
        self.rebuild_pending.store(true, Ordering::Relaxed);
        let reverse_dependencies = self
            .build_record
            .read()
            .unwrap()
            .reverse_dependencies
            .clone();

        for rev in reverse_dependencies {
            if let Some(rev) = rev.upgrade() {
                rev.invalidate();
            }
        }
    }
}

impl Hash for LazyPayload {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.worker.identity());
    }
}

impl PartialEq for LazyPayload {
    fn eq(&self, other: &Self) -> bool {
        self.worker.identity() == other.worker.identity()
    }
}
impl Eq for LazyPayload {}

impl Clone for LazyPayload {
    fn clone(&self) -> Self {
        Self {
            worker: self.worker.clone_boxed(),
            build_record: Default::default(),
            rebuild_pending: AtomicBool::new(true),
        }
    }
}

pub type BuildDependency = Arc<LazyPayload>;
pub type ReverseBuildDependency = Weak<LazyPayload>;

#[derive(Default)]
pub struct BuildRecord {
    artifact: Option<Arc<dyn Any + Send + Sync>>,

    // Assets this one requested during the last build
    pub dependencies: HashSet<BuildDependency>,
    // Assets that requested this asset during their builds
    pub reverse_dependencies: Vec<ReverseBuildDependency>,
}

pub(crate) struct BuildRecordDiff {
    pub added_deps: Vec<BuildDependency>,
    pub removed_deps: Vec<BuildDependency>,
}

enum LazyInner {
    Cached(Arc<LazyPayload>),
    Isolated(Arc<dyn LazyWorkerObj>),
}

impl Clone for LazyInner {
    fn clone(&self) -> Self {
        match self {
            Self::Cached(cached) => Self::Cached(cached.clone()),
            Self::Isolated(isolated) => Self::Isolated(isolated.clone()),
        }
    }
}

pub struct Lazy<T: LazyReqs> {
    inner: Mutex<LazyInner>,
    identity: u64,
    pub debug_name: &'static str,
    marker: PhantomData<T>,
}

impl<T: LazyReqs> Clone for Lazy<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Mutex::new(self.inner.lock().unwrap().clone()),
            identity: self.identity,
            debug_name: self.debug_name,
            marker: PhantomData,
        }
    }
}

impl<T: LazyReqs> Hash for Lazy<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.identity.hash(state);
    }
}

pub(crate) struct EvalTracker {
    pub dependencies: Mutex<HashSet<BuildDependency>>,
    pub current_ref: Arc<LazyPayload>,
}

impl EvalTracker {
    fn new(current_ref: Arc<LazyPayload>) -> Self {
        Self {
            dependencies: Default::default(),
            current_ref,
        }
    }
}

#[derive(Clone)]
pub struct RunContext {
    pub(crate) cache: Arc<Cache>,
    pub(crate) tracker: Option<Arc<EvalTracker>>,
}

impl RunContext {
    pub fn get_invalidation_trigger(&self) -> impl Fn() + Send + Sync {
        let current_ref = Arc::downgrade(&self.tracker.as_ref().unwrap().current_ref);
        move || {
            if let Some(current_ref) = current_ref.upgrade() {
                current_ref.invalidate();
            }
        }
    }
}

impl RunContext {
    fn register_dependency(&self, dep: &Arc<LazyPayload>) {
        if let Some(tracker) = self.tracker.as_ref() {
            println!(
                "    Registering a dependency on {}",
                dep.worker.debug_name()
            );
            tracker.dependencies.lock().unwrap().insert(dep.clone());
        }
    }
}

impl From<Arc<Cache>> for RunContext {
    fn from(cache: Arc<Cache>) -> Self {
        RunContext {
            cache,
            tracker: None,
        }
    }
}

impl From<&Arc<Cache>> for RunContext {
    fn from(cache: &Arc<Cache>) -> Self {
        RunContext {
            cache: cache.clone(),
            tracker: None,
        }
    }
}

type BoxedFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

impl<T: LazyReqs> Lazy<T> {
    pub fn eval(&self, context: impl Into<RunContext>) -> BoxedFuture<Result<Arc<T>>> {
        let context = context.into();

        let payload = {
            let mut inner = self.inner.lock().unwrap();

            match &mut *inner {
                LazyInner::Cached(cached) => cached.clone(),
                LazyInner::Isolated(isolated) => {
                    let worker = isolated.clone_boxed();
                    let type_id = TypeId::of::<T>();
                    let cached =
                        context
                            .cache
                            .get_or_insert_with(type_id, self.identity, move || LazyPayload {
                                worker,
                                build_record: Default::default(),
                                rebuild_pending: AtomicBool::new(true),
                            });

                    let result = cached.clone();

                    // Connect to cache, and return the cached payload
                    *inner = LazyInner::Cached(cached);
                    result
                }
            }
        };

        context.register_dependency(&payload);

        if payload.rebuild_pending.load(Ordering::Relaxed) {
            let worker = payload.worker.clone_boxed();
            let context = RunContext {
                cache: context.cache,
                tracker: Some(Arc::new(EvalTracker::new(payload.clone()))),
            };

            log::info!("Evaluating {}", self.debug_name);

            Box::pin(async move {
                // Clear rebuild pending status before running the worker.
                // If the asset becomes invalidated while the worker is running,
                // it will need to be evaluated again next time.

                payload.rebuild_pending.store(false, Ordering::Relaxed);

                let tracker = context.tracker.as_ref().unwrap().clone();
                let worker = worker.run_boxed(context);
                let res: Arc<dyn Any + Send + Sync> = tokio::task::spawn(worker).await??;

                let build_record_diff = payload.set_new_build_result(
                    Some(res),
                    Arc::try_unwrap(tracker)
                        .ok()
                        .expect("EvalTracker references cannot be retained")
                        .dependencies
                        .into_inner()
                        .unwrap(),
                );

                for dep in &build_record_diff.removed_deps {
                    let dep = &dep.build_record;
                    let mut dep = dep.write().unwrap();
                    let to_remove: *const LazyPayload = &*payload;

                    dep.reverse_dependencies.retain(|r| {
                        let r = r.as_ptr();
                        !r.is_null() && !std::ptr::eq(r, to_remove)
                    });
                }

                for dep in &build_record_diff.added_deps {
                    let dep = &dep.build_record;
                    let mut dep = dep.write().unwrap();
                    let to_add: *const LazyPayload = &*payload;

                    let exists = dep
                        .reverse_dependencies
                        .iter()
                        .any(|r| std::ptr::eq(r.as_ptr(), to_add));

                    if !exists {
                        dep.reverse_dependencies.push(Arc::downgrade(&payload));
                    }
                }

                if payload.rebuild_pending.load(Ordering::Relaxed) {
                    // The result was invalidated while the worker was running. Invalidate the new build record too.
                    payload.invalidate();
                }

                let build_record = payload.build_record.read().unwrap();
                let v: Option<Arc<T>> = build_record
                    .artifact
                    .clone()
                    .map(|artifact| Arc::downcast::<T>(artifact).expect("downcast"));

                if let Some(v) = v {
                    Ok(v)
                } else {
                    Err(anyhow!("The requested asset failed to build"))
                }
            })
        } else {
            let build_record = payload.build_record.read().unwrap();
            let v: Option<Arc<T>> = build_record
                .artifact
                .clone()
                .map(|artifact| Arc::downcast::<T>(artifact).expect("downcast"));
            let v: Arc<T> = v.expect("a valid build result");
            Box::pin(async move { Ok(v) })
        }
    }
}

pub trait ToLazyIdentity {
    fn identity(&self) -> u64;
}

impl<T: Hash> ToLazyIdentity for T {
    fn identity(&self) -> u64 {
        let mut s = twox_hash::XxHash64::default();
        <Self as std::hash::Hash>::hash(&self, &mut s);
        s.finish()
    }
}

pub trait IntoLazy: ToLazyIdentity
where
    Self: LazyWorker + Sized + Clone + Hash,
{
    fn into_lazy(self) -> Lazy<<Self as LazyWorker>::Output> {
        let identity = self.identity();

        Lazy {
            identity,
            inner: Mutex::new(LazyInner::Isolated(Arc::new(self))),
            debug_name: std::any::type_name::<Self>(),
            marker: PhantomData,
        }
    }
}
