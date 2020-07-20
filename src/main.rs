//#![feature(associated_type_defaults)]
//#![allow(unused_imports)]

#[allow(unused_imports)]
use anyhow::{anyhow, Result};

use async_trait::async_trait;
use std::sync::Arc;
use tokio::runtime::Runtime;

mod cache;
mod lazy;

use cache::*;
use lazy::*;

impl ToLazy for i32 {
    fn identity(&self) -> u64 {
        *self as u64 // TODO: hash
    }
}

#[async_trait]
impl LazyWorker for i32 {
    type Output = i32;

    async fn run(self, _: Cache) -> Result<Self::Output> {
        Ok(self)
    }
}

#[derive(Clone)]
struct Add {
    a: Arc<Lazy<i32>>,
    b: Arc<Lazy<i32>>,
}

impl ToLazy for Add {
    fn identity(&self) -> u64 {
        self.a.identity * 12345 + self.b.identity // TODO: hash
    }
}

#[async_trait]
impl LazyWorker for Add {
    type Output = i32;

    async fn run(self, cache: Cache) -> Result<Self::Output> {
        let a = self.a.eval(&cache).await?;
        let b = self.b.eval(&cache).await?;
        println!("running Add({}, {})", *a, *b);
        Ok(*a + *b)
    }
}

#[derive(Clone)]
struct MutAdd {
    a: Lazy<i32>,
    b: Arc<Lazy<i32>>,
}

impl ToLazy for MutAdd {
    fn identity(&self) -> u64 {
        self.a.identity * 12345 + self.b.identity // TODO: hash
    }
}

#[async_trait]
impl LazyWorker for MutAdd {
    type Output = ();

    async fn run(self, cache: Cache) -> Result<Self::Output> {
        let b = self.b.clone().eval(&cache).await?;
        let mut a = self.a.eval(&cache).await?;
        println!("running MutAdd({}, {})", *a, *b);
        *a += *b;
        Ok(())
    }
}

fn try_main() -> Result<()> {
    let a = 1i32.lazy().shared();
    let b = 2i32.lazy().shared();
    let c = Add { a, b }.lazy().shared();

    let cache = CacheDb::create();
    let mut runtime = Runtime::new()?;

    // Should return two references:
    // A shared immutable ref to the previous value. Its purpose is to say "you can read
    // the previous value, but you don't own it, and can't rebind it." This reference
    // should only be valid until the second returned value is used.
    // The purpose of the second returned value is to allow rebinding the temporal resource.
    // There's probably no way to make this work with static verification, so runtime checks.
    // Note: mutation is not allowed for any of the lazy values. The way GPU rendering
    // with in-place changes could work is through interior mutability, relying on the
    // locking inherent in using a single render/compute queue.
    let (f_prev, f_next) = 0i32.lazy().feedback();

    // Note: f_prev must be used within the eval tree of f below. Otherwise
    // it might be evaluated out of order, and get the latest value instead of the prev one.
    let d = Add {
        a: c.clone(),
        b: f_prev,
    }
    .lazy();

    let f = f_next.rebind(|feedback| MutAdd {
        a: feedback,
        b: 1i32.lazy().shared(),
    });

    dbg!(*runtime.block_on(c.eval(&cache))?);
    dbg!(runtime.block_on(d.eval(&cache))?);
    dbg!(runtime.block_on(f.clone().eval(&cache))?);

    Ok(())
}

fn main() {
    if let Err(err) = try_main() {
        eprintln!("ERROR: {:?}", err);
        err.chain()
            .skip(1)
            .for_each(|cause| eprintln!("because: {}", cause));
        std::process::exit(1);
    }
}

// Two kinds of feedback:
// * mutable ref: in-place update a resource
//     * rebind nominal value without creating a copy
//     * quite special; can't include itself in eval tree
// * double-backed: current and previous values are distinct instances
//     * TAA and company
//     * regular temporal loop; publishes a new value in every iteration

// Can we use the Rust borrow checker to validate usage?

struct Thing {
    ver: i32,
}

fn mut_thing(a: &mut Thing) {
    a.ver += 1;
}

fn use_thing(a: &Thing) {}

fn proto() {
    let mut a = Thing { ver: 0 };
    mut_thing(&mut a);
    use_thing(&a);
    mut_thing(&mut a);
    use_thing(&a);
    use_thing(&a);
}
