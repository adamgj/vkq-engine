// `loom` is a compile-time cfg set through RUSTFLAGS (`--cfg loom`), not a
// cargo feature, so declare it to the unexpected_cfgs lint.
fn main() {
    println!("cargo::rustc-check-cfg=cfg(loom)");
}
