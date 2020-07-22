mod cache;
mod lazy;

pub use anyhow::{anyhow, Result};
pub use async_trait::async_trait;
pub use cache::*;
pub use lazy::*;

/*
fn main() {
    use chrono::Local;
    use env_logger::Builder;
    use log::LevelFilter;
    use std::io::Write;

    let mut builder = Builder::new();
    builder.format(|buf, record| {
        writeln!(
            buf,
            "{} [{}] - {}",
            Local::now().format("%H:%M:%S"),
            record.level(),
            record.args()
        )
    });
    Builder::filter(&mut builder, None, LevelFilter::Trace);
    builder.init();

    if let Err(err) = try_main() {
        eprintln!("ERROR: {:?}", err);
        err.chain()
            .skip(1)
            .for_each(|cause| eprintln!("because: {}", cause));
        std::process::exit(1);
    }
}
*/
