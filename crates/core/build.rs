//! The core's Rust records are generated from the Nexus's Ethos by
//! ethos-zero and committed; every build regenerates them and refuses a
//! committed file that differs.
use ethos_zero::{Actualizing, File, Generating, Potential};

fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest"));
    let ethos = manifest.join("../../ethos");
    for name in ["memory", "operation"] {
        let source_path = ethos.join(format!("{name}.ethos"));
        let generated_path = manifest.join(format!("src/generated/{name}.rs"));
        println!("cargo:rerun-if-changed={}", source_path.display());
        println!("cargo:rerun-if-changed={}", generated_path.display());
        let source = std::fs::read_to_string(&source_path).expect("source");
        let file = Potential::<File>::from(source)
            .actualize()
            .unwrap_or_else(|_| panic!("read {}", source_path.display()));
        let generated = file
            .generate()
            .unwrap_or_else(|_| panic!("generate {}", source_path.display()));
        assert_eq!(
            generated,
            std::fs::read_to_string(&generated_path).expect("generated"),
            "{} is not what ethos-zero generates from {}",
            generated_path.display(),
            source_path.display()
        );
    }
}
