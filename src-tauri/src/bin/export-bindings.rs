use std::path::PathBuf;

fn main() {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("src/bindings.ts"));
    dbox_lib::api::export_bindings(&output).expect("TypeScript bindings should be generated");
}
