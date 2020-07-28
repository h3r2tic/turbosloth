use tokio::runtime::Runtime;
use turbosloth::*;

#[derive(Clone, Hash, IntoLazy)]
struct Add {
    a: i32,
    b: i32,
}

#[async_trait]
impl LazyWorker for Add {
    type Output = i32;
    type Error = anyhow::Error;

    async fn run(self, _: RunContext) -> anyhow::Result<Self::Output> {
        println!("Running Add({}, {})", self.a, self.b);
        Ok(self.a + self.b)
    }
}

fn main() -> anyhow::Result<()> {
    let cache = LazyCache::create();
    let mut runtime = Runtime::new()?;

    {
        let add1 = Add { a: 5, b: 7 }.into_lazy();
        let add2 = Add { a: 5, b: 7 }.into_lazy();
        let add3 = add2.clone();

        dbg!(*runtime.block_on(add1.eval(&cache))?);
        dbg!(*runtime.block_on(add2.eval(&cache))?);
        dbg!(*runtime.block_on(add3.eval(&cache))?);
        dbg!(*runtime.block_on(Add { a: 5, b: 7 }.into_lazy().eval(&cache))?);
        dbg!(*runtime.block_on(Add { a: 5, b: 8 }.into_lazy().eval(&cache))?);
    }

    // Lose the above references, thus removing them from the cache

    {
        dbg!(*runtime.block_on(Add { a: 5, b: 7 }.into_lazy().eval(&cache))?);
    }

    Ok(())
}
