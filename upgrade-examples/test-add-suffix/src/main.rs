use snafu::Snafu;

#[derive(Debug, Snafu)]
enum EnumError {
    EnumVariant1,
    EnumVariant2 { name: String },
}

fn main() {
    let _ = EnumVariant1.build();
    let _ = EnumVariant2 { name: "name" }.build();
}
