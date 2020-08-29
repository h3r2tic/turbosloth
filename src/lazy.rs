use crate::cache::*;

use async_trait::async_trait;
use std::{
    any::{Any, TypeId},
    collections::HashSet,
    error::Error,
    future::Future,
    hash::{Hash, Hasher},
    marker::PhantomData,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock, Weak,
    },
};

#[derive(Debug, Clone)]
pub struct LazyEvalError {
    pub worker_debug_name: &'static str,
    pub source: Arc<dyn Error + Send + Sync + 'static>,
}

impl std::error::Error for LazyEvalError {
    fn source(&self) -> std::option::Option<&(dyn Error + 'static)> {
        Some(&*self.source)
    }
}

impl std::fmt::Display for LazyEvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "A turbosloth LazyWorker \"{}\" failed",
            self.worker_debug_name
        )
    }
}

pub trait LazyReqs: Any + Sized + Send + Sync + 'static {}
impl<T: Any + Sized + Send + Sync + 'static> LazyReqs for T {}

#[async_trait]
pub trait LazyWorker: Send + Sync + 'static {
    type Output;

    async fn run(self, ctx: RunContext) -> Self::Output;
}

pub trait LazyWorkerImpl {
    type Value: Send + Sync + 'static;
    type Error: Into<Box<dyn Error + 'static + Sync + Send>>;

    fn run(self, ctx: RunContext) -> BoxedWorkerFuture;
}

impl<T: LazyReqs, E, W> LazyWorkerImpl for W
where
    W: LazyWorker<Output = std::result::Result<T, E>> + Clone + Hash,
    E: Into<Box<dyn Error + 'static + Sync + Send>>,
{
    type Value = T;
    type Error = E;

    fn run(self, ctx: RunContext) -> BoxedWorkerFuture {
        Box::pin(async {
            <Self as LazyWorker>::run(self, ctx)
                .await
                .map(|result| -> Arc<dyn Any + Send + Sync> { Arc::new(result) })
                .map_err(|err| err.into())
        })
    }
}

type BoxedWorkerFuture = Pin<
    Box<
        dyn Future<
                Output = std::result::Result<
                    Arc<dyn Any + Send + Sync>,
                    Box<dyn Error + Send + Sync + 'static>,
                >,
            > + Send
            + 'static,
    >,
>;

pub trait LazyWorkerObj: Send + Sync {
    fn identity(&self) -> u64;
    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj>;
    fn run_boxed(self: Box<Self>, context: RunContext) -> BoxedWorkerFuture;
    fn debug_name(&self) -> &'static str;
}

impl<T: LazyReqs, E, W> LazyWorkerObj for W
where
    W: LazyWorker + Clone + Hash,
    W: LazyWorkerImpl<Value = T, Error = E>,
    E: Into<Box<dyn Error + 'static + Sync + Send>>,
{
    fn identity(&self) -> u64 {
        <Self as LazyIdentity>::lazy_identity(self)
    }

    fn clone_boxed(&self) -> Box<dyn LazyWorkerObj> {
        Box::new((*self).clone())
    }

    fn run_boxed(self: Box<Self>, context: RunContext) -> BoxedWorkerFuture {
        <Self as LazyWorkerImpl>::run(*self, context)
    }

    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

pub struct LazyPayload {
    worker: Box<dyn LazyWorkerObj>,
    build_record: RwLock<BuildRecord>,
    rebuild_pending: AtomicBool,
}

impl LazyPayload {
    fn set_new_build_result(
        &self,
        artifact: BuildArtifact,
        dependencies: HashSet<BuildDependency>,
    ) -> BuildRecordDiff {
        let mut build_record = self.build_record.write().unwrap();

        let prev_deps = std::mem::take(&mut build_record.dependencies);

        let added_deps = dependencies.difference(&prev_deps).cloned().collect();
        let removed_deps = prev_deps.difference(&dependencies).cloned().collect();

        build_record.dependencies = dependencies;
        build_record.artifact = Some(artifact);

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

type BuildDependency = Arc<LazyPayload>;
type ReverseBuildDependency = Weak<LazyPayload>;
type BuildArtifact = std::result::Result<Arc<dyn Any + Send + Sync>, LazyEvalError>;

#[derive(Default)]
struct BuildRecord {
    artifact: Option<BuildArtifact>,

    // Assets this one requested during the last build
    dependencies: HashSet<BuildDependency>,
    // Assets that requested this asset during their builds
    reverse_dependencies: Vec<ReverseBuildDependency>,
}

pub(crate) struct BuildRecordDiff {
    pub added_deps: Vec<BuildDependency>,
    pub removed_deps: Vec<BuildDependency>,
}

pub enum OpaqueLazy {
    Cached(Arc<LazyPayload>),
    Isolated(Arc<dyn LazyWorkerObj>),
}

impl OpaqueLazy {
    pub fn is_up_to_date(&self) -> bool {
        match self {
            OpaqueLazy::Cached(payload) => !payload.rebuild_pending.load(Ordering::Relaxed),
            OpaqueLazy::Isolated(..) => false,
        }
    }
}

impl Clone for OpaqueLazy {
    fn clone(&self) -> Self {
        match self {
            Self::Cached(cached) => Self::Cached(cached.clone()),
            Self::Isolated(isolated) => Self::Isolated(isolated.clone()),
        }
    }
}

pub struct Lazy<T: LazyReqs> {
    inner: RwLock<OpaqueLazy>,
    identity: u64,
    pub debug_name: &'static str,
    marker: PhantomData<T>,
}

impl<T: LazyReqs> Lazy<T> {
    fn new(identity: u64, worker: Arc<dyn LazyWorkerObj>, debug_name: &'static str) -> Self {
        Self {
            inner: RwLock::new(OpaqueLazy::Isolated(worker)),
            identity,
            debug_name,
            marker: PhantomData,
        }
    }

    pub fn identity(&self) -> u64 {
        self.identity
    }
}

impl<T: LazyReqs> Clone for Lazy<T> {
    fn clone(&self) -> Self {
        Self {
            inner: RwLock::new(self.inner.read().unwrap().clone()),
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
    pub(crate) cache: Arc<LazyCache>,
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
            /*tracing::trace!(
                "{}: Registering a dependency on {}",
                tracker.current_ref.worker.debug_name(),
                dep.worker.debug_name()
            );*/
            tracker.dependencies.lock().unwrap().insert(dep.clone());
        }
    }
}

impl From<Arc<LazyCache>> for RunContext {
    fn from(cache: Arc<LazyCache>) -> Self {
        RunContext {
            cache,
            tracker: None,
        }
    }
}

impl From<&Arc<LazyCache>> for RunContext {
    fn from(cache: &Arc<LazyCache>) -> Self {
        RunContext {
            cache: cache.clone(),
            tracker: None,
        }
    }
}

/*impl From<&RunContext> for RunContext {
    fn from(ctx: &RunContext) -> Self {
        RunContext {
            cache: ctx.cache.clone(),
            tracker: ctx.tracker.clone(),
        }
    }
}*/

pub trait AsRunContext {
    fn as_run_context(&self) -> RunContext;
}

impl AsRunContext for RunContext {
    fn as_run_context(&self) -> RunContext {
        self.clone()
    }
}

impl AsRunContext for Arc<LazyCache> {
    fn as_run_context(&self) -> RunContext {
        RunContext {
            cache: self.clone(),
            tracker: None,
        }
    }
}

impl<T: LazyReqs> Lazy<T> {
    pub fn is_up_to_date(&self) -> bool {
        let inner = self.inner.read().unwrap();
        inner.is_up_to_date()
    }

    pub fn is_stale(&self) -> bool {
        let inner = self.inner.read().unwrap();
        !inner.is_up_to_date()
    }

    pub fn into_opaque(self) -> OpaqueLazy {
        self.inner.into_inner().unwrap()
    }

    pub fn eval(
        &self,
        ctx: &impl AsRunContext,
    ) -> impl Future<Output = std::result::Result<Arc<T>, LazyEvalError>> {
        let ctx: RunContext = ctx.as_run_context();

        let payload = {
            let mut inner = self.inner.write().unwrap();

            match &mut *inner {
                OpaqueLazy::Cached(cached) => cached.clone(),
                OpaqueLazy::Isolated(isolated) => {
                    let worker = isolated.clone_boxed();
                    let type_id = TypeId::of::<T>();
                    let cached = ctx
                        .cache
                        .get_or_insert_with(type_id, self.identity, move || LazyPayload {
                            worker,
                            build_record: Default::default(),
                            rebuild_pending: AtomicBool::new(true),
                        });

                    let result = cached.clone();

                    // Connect to cache, and return the cached payload
                    *inner = OpaqueLazy::Cached(cached);
                    result
                }
            }
        };

        ctx.register_dependency(&payload);
        let worker_debug_name = self.debug_name;

        async move {
            if payload.rebuild_pending.load(Ordering::Relaxed) {
                let worker = payload.worker.clone_boxed();
                let context = RunContext {
                    cache: ctx.cache,
                    tracker: Some(Arc::new(EvalTracker::new(payload.clone()))),
                };

                // tracing::info!("Evaluating {}", debug_name);

                // Clear rebuild pending status before running the worker.
                // If the asset becomes invalidated while the worker is running,
                // it will need to be evaluated again next time.

                payload.rebuild_pending.store(false, Ordering::Relaxed);

                let tracker = context.tracker.as_ref().unwrap().clone();
                let worker = worker.run_boxed(context);
                let build_artifact: BuildArtifact =
                    worker
                        .await
                        .map_err(
                            |err: Box<dyn Error + Send + Sync + 'static>| LazyEvalError {
                                worker_debug_name,
                                source: err.into(),
                            },
                        );

                let build_record_diff = payload.set_new_build_result(
                    build_artifact,
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
                build_record
                    .artifact
                    .clone()
                    .unwrap()
                    .map(|artifact| Arc::downcast::<T>(artifact).expect("downcast"))
            } else {
                let build_record = payload.build_record.read().unwrap();
                build_record
                    .artifact
                    .clone()
                    .unwrap()
                    .map(|artifact| Arc::downcast::<T>(artifact).expect("downcast"))
            }
        }
    }
}

pub trait LazyIdentity {
    fn lazy_identity(&self) -> u64;
}

impl<T: Hash> LazyIdentity for T {
    fn lazy_identity(&self) -> u64 {
        let mut s = wyhash::WyHash::default();
        <Self as std::hash::Hash>::hash(&self, &mut s);
        s.finish()
    }
}

pub trait IntoLazy: LazyIdentity
where
    Self: Clone + Hash + Sized + LazyIdentity + LazyWorker + LazyWorkerImpl,
{
    fn into_lazy(self) -> crate::lazy::Lazy<<Self as crate::lazy::LazyWorkerImpl>::Value> {
        let identity = <Self as crate::lazy::LazyIdentity>::lazy_identity(&self);

        Lazy::new(
            identity,
            std::sync::Arc::new(self),
            std::any::type_name::<Self>(),
        )
    }
}

impl<W> IntoLazy for W where W: Clone + Hash + Sized + LazyIdentity + LazyWorker + LazyWorkerImpl {}
