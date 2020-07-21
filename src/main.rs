#[allow(unused_imports)]
use anyhow::{anyhow, Result};

use async_trait::async_trait;
use std::sync::Arc;
use tokio::runtime::Runtime;

mod cache;
mod lazy;

use cache::*;
use lazy::*;

#[derive(Clone, Hash, IntoLazy)]
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

fn try_main() -> Result<()> {
    let add1 = Add { a: 5, b: 7 }.into_lazy();
    let add2 = Add { a: 5, b: 7 }.into_lazy();
    let add3 = add2.clone();

    let cache = Cache::create();
    let mut runtime = Runtime::new()?;

    dbg!(*runtime.block_on(add1.eval(&cache))?);
    dbg!(*runtime.block_on(add2.eval(&cache))?);
    dbg!(*runtime.block_on(add3.eval(&cache))?);
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
