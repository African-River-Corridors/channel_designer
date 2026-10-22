//! Record the git tags the workspace pins its engines to (ORD, otd, arc-rules), so the run report
//! can print them. The crate versions do not say which tag was built: the drawing crate's version
//! is not its tag. Read at build time from the workspace manifest; the library itself stays pure.

use std::path::Path;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).unwrap_or_default();
    for (dep, var) in [
        ("ord", "OWD_PIN_ORD"),
        ("otd", "OWD_PIN_OTD"),
        ("arc-rules", "OWD_PIN_ARC_RULES"),
    ] {
        let tag = text
            .lines()
            .find(|l| l.trim_start().starts_with(&format!("{dep} = ")))
            .and_then(|l| l.split("tag = \"").nth(1))
            .and_then(|r| r.split('"').next())
            .unwrap_or("unknown");
        println!("cargo:rustc-env={var}={tag}");
    }
}
