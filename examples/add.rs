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

    async fn run(self, _: LazyContext) -> Result<Self::Output> {
        dbg!(self.identity());
        Ok(self.a + self.b)
    }
}

fn main() -> Result<()> {
    let add1 = Add { a: 5, b: 7 }.into_lazy();
    let add2 = Add { a: 5, b: 7 }.into_lazy();
    let add3 = add2.clone();

    let cache = Cache::create();
    let mut runtime = Runtime::new()?;

    dbg!(*runtime.block_on(add1.eval(&cache))?);
    dbg!(*runtime.block_on(add2.eval(&cache))?);
    dbg!(*runtime.block_on(add3.eval(&cache))?);
    dbg!(*runtime.block_on(Add { a: 5, b: 7 }.into_lazy().eval(&cache))?);
    dbg!(*runtime.block_on(Add { a: 5, b: 8 }.into_lazy().eval(&cache))?);

    Ok(())
}
