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

#[async_trait]
impl LazyWorker for i32 {
    type Output = i32;

    async fn run(self, _: Arc<Cache>) -> Result<Self::Output> {
        Ok(self)
    }
}
impl IntoLazy for i32 {}

#[derive(Clone, Hash)]
struct AddLazy {
    a: Lazy<i32>,
    b: Lazy<i32>,
}

#[async_trait]
impl LazyWorker for AddLazy {
    type Output = i32;

    async fn run(self, cache: Arc<Cache>) -> Result<Self::Output> {
        let a = self.a.eval(&cache).await?;
        let b = self.b.eval(&cache).await?;
        println!("running AddLazy({}, {})", *a, *b);
        Ok(*a + *b)
    }
}
impl IntoLazy for AddLazy {}

#[derive(Clone, Hash)]
struct Add {
    a: i32,
    b: i32,
}

#[async_trait]
impl LazyWorker for Add {
    type Output = i32;

    async fn run(self, _: Arc<Cache>) -> Result<Self::Output> {
        println!("running Add({}, {})", self.a, self.b);
        Ok(self.a + self.b)
    }
}
impl IntoLazy for Add {}

fn try_main() -> Result<()> {
    let a = 1i32.into_lazy();
    let b = 2i32.into_lazy();
    let c = AddLazy { a, b }.into_lazy();

    let d1 = Add { a: 5, b: 7 }.into_lazy();
    let d2 = Add { a: 5, b: 7 }.into_lazy();
    let d3 = d2.clone();

    let cache = Cache::create();
    let mut runtime = Runtime::new()?;

    dbg!(*runtime.block_on(c.eval(&cache))?);
    dbg!(*runtime.block_on(c.eval(&cache))?);
    dbg!(*runtime.block_on(d1.eval(&cache))?);
    dbg!(*runtime.block_on(d2.eval(&cache))?);
    dbg!(*runtime.block_on(d3.eval(&cache))?);

    dbg!(*runtime.block_on(Add { a: 5, b: 7 }.into_lazy().eval(&cache))?);

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
