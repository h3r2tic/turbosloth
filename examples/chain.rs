use tokio::runtime::Runtime;
use turbosloth::*;

#[derive(Clone, Hash)]
struct Op1(i32);

#[async_trait]
impl LazyWorker for Op1 {
    type Output = anyhow::Result<i32>;

    async fn run(self, _: RunContext) -> Self::Output {
        println!("Running Op1");
        Ok(self.0 * 10)
    }
}

#[derive(Clone, Hash)]
struct Op2(Lazy<i32>);

#[async_trait]
impl LazyWorker for Op2 {
    type Output = anyhow::Result<i32>;

    async fn run(self, ctx: RunContext) -> Self::Output {
        println!("Running Op2");
        Ok(*self.0.eval(&ctx).await? + 7)
    }
}

fn main() -> anyhow::Result<()> {
    let cache = LazyCache::create();
    let mut runtime = Runtime::new()?;

    let op12 = Op2(Op1(1).into_lazy()).into_lazy();
    dbg!(*runtime.block_on(op12.eval(&cache))?);

    Ok(())
}
