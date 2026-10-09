//! A no-op unless `--features doom` or `micropolis`, so the default build
//! needs no C or C++ compiler.
//!
//! With it, doomgeneric, `doom/afterglow_doom.c` and the autopilot
//! (`doom/autopilot.c`, `doom/ap_nav.c`, `doom/ap_effect.c`) are compiled into one static library. `exit` is defined to `dg_exit`, which is how the engine's
//! error paths reach the glue's setjmp boundary instead of ending the process.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(feature = "doom")]
    doom();
    #[cfg(feature = "micropolis")]
    micropolis();
}

/// The Micropolis engine and `micropolis/afterglow_micropolis.cpp`, its C API
/// and crash boundary, as one static library. Exceptions and RTTI are off:
/// the engine uses neither, and the boundary is setjmp. libstdc++ is linked
/// statically on musl so the image stays one static binary.
#[cfg(feature = "micropolis")]
fn micropolis() {
    println!("cargo::rerun-if-changed=micropolis");
    let mut sources: Vec<std::path::PathBuf> = std::fs::read_dir("micropolis/engine")
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "cpp"))
        .collect();
    sources.sort();
    sources.push("micropolis/afterglow_micropolis.cpp".into());
    let musl = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|e| e == "musl");
    let mut b = cc::Build::new();
    b.cpp(true)
        .files(&sources)
        .include("micropolis/engine")
        .std("c++11")
        .opt_level(2)
        .warnings(false)
        .flag_if_supported("-w")
        .flag("-fno-exceptions")
        .flag("-fno-rtti");
    if musl {
        b.cpp_link_stdlib(None);
    }
    b.compile("micropolis");
    if musl {
        println!("cargo::rustc-link-lib=static=stdc++");
    }
}

#[cfg(feature = "doom")]
fn doom() {
    println!("cargo::rerun-if-changed=doom");
    let mut sources: Vec<std::path::PathBuf> = std::fs::read_dir("doom/doomgeneric")
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "c"))
        .collect();
    sources.sort();
    sources.extend(
        [
            "doom/afterglow_doom.c",
            "doom/autopilot.c",
            "doom/ap_nav.c",
            "doom/ap_effect.c",
        ]
        .map(Into::into),
    );
    cc::Build::new()
        .files(&sources)
        .include("doom/doomgeneric")
        .define("CMAP256", None)
        .define("DOOMGENERIC_RESX", "320")
        .define("DOOMGENERIC_RESY", "200")
        .define("NORMALUNIX", None)
        .define("LINUX", None)
        .define("_DEFAULT_SOURCE", None)
        .define("exit", "dg_exit")
        .opt_level(2)
        .warnings(false)
        .flag_if_supported("-w")
        .flag("-fno-common")
        .flag("-fno-strict-aliasing")
        .compile("doomgeneric");
}
