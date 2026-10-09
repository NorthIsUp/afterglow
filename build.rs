//! A no-op unless `--features doom`, so the default build needs no C compiler.
//!
//! With it, doomgeneric is compiled once per engine instance. Doom keeps its
//! whole world in C globals, so two maps at once means two copies of every
//! global: each copy is built with a forced-include header that renames every
//! global symbol the engine defines (found with `nm` on an unrenamed scan
//! build, so the list cannot go stale) to `dg<N>_<name>`. `exit` is renamed
//! too, which is how the engine's error paths reach `doom/afterglow_doom.c`'s
//! setjmp boundary instead of ending the process.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(feature = "doom")]
    doom::build();
}

#[cfg(feature = "doom")]
mod doom {
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// Must match `INSTANCES` in src/doom/engine.rs.
    const INSTANCES: usize = 4;

    fn base(sources: &[PathBuf]) -> cc::Build {
        let mut b = cc::Build::new();
        b.files(sources)
            .include("doom/doomgeneric")
            .define("CMAP256", None)
            .define("DOOMGENERIC_RESX", "320")
            .define("DOOMGENERIC_RESY", "200")
            .define("NORMALUNIX", None)
            .define("LINUX", None)
            .define("_DEFAULT_SOURCE", None)
            .opt_level(2)
            .warnings(false)
            .flag_if_supported("-w")
            .flag("-fno-common")
            .flag("-fno-strict-aliasing");
        b
    }

    pub fn build() {
        println!("cargo::rerun-if-changed=doom");
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let mut sources: Vec<PathBuf> = std::fs::read_dir("doom/doomgeneric")
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "c"))
            .collect();
        sources.sort();
        sources.push("doom/afterglow_doom.c".into());

        let scan = base(&sources)
            .out_dir(out.join("scan"))
            .compile_intermediates();
        let names = globals(&scan);
        assert!(
            names.len() > 100,
            "nm found only {} doom globals",
            names.len()
        );

        for i in 0..INSTANCES {
            let header = out.join(format!("dg{i}.h"));
            let mut text = String::new();
            for n in names.iter().map(String::as_str).chain(["exit"]) {
                writeln!(text, "#define {n} dg{i}_{n}").unwrap();
            }
            std::fs::write(&header, text).unwrap();
            base(&sources)
                .flag("-include")
                .flag(header.to_str().unwrap())
                .out_dir(out.join(format!("dg{i}")))
                .compile(&format!("dg{i}"));
        }
    }

    /// Every global symbol the objects define, unmangled.
    fn globals(objects: &[PathBuf]) -> Vec<String> {
        let nm = std::env::var("NM").unwrap_or_else(|_| "nm".into());
        let apple = std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple");
        let mut names = Vec::new();
        for o in objects {
            names.extend(defined(&nm, o, apple));
        }
        names.sort();
        names.dedup();
        names
    }

    fn defined(nm: &str, object: &Path, apple: bool) -> Vec<String> {
        let out = Command::new(nm)
            .args(["-g", "--defined-only", "-P"])
            .arg(object)
            .output()
            .unwrap_or_else(|e| panic!("running {nm}: {e}"));
        assert!(out.status.success(), "{nm} failed on {}", object.display());
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .map(|n| {
                if apple {
                    n.strip_prefix('_').unwrap_or(n)
                } else {
                    n
                }
                .to_string()
            })
            .filter(|n| n != "main")
            .collect()
    }
}
