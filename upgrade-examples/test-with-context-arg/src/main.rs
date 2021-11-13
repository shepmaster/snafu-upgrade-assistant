use snafu::{Snafu, ResultExt};

#[derive(Debug, Snafu)]
struct Inner;

#[derive(Debug, Snafu)]
enum Error {
    Variant1 { source: Inner },
}

fn main() {
    let inner = InnerContext.fail::<()>();
    let _ = inner.with_context(|| Variant1);
}
