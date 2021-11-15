use snafu::Snafu;

#[derive(Debug, Snafu)]
#[snafu(visibility = "pub(crate)")]
enum Error {
    Variant1,
}

fn main() {
    let _ = Variant1.build();
}
