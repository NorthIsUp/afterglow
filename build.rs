//! A no-op unless `--features doom`, so the default build needs no C compiler.
//!
//! With it, doomgeneric, `doom/afterglow_doom.c` and the autopilot
//! (`doom/autopilot.c`, `doom/ap_nav.c`) are compiled into one static library. `exit` is defined to `dg_exit`, which is how the engine's
//! error paths reach the glue's setjmp boundary instead of ending the process.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    #[cfg(feature = "doom")]
    doom();
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
    sources.extend(["doom/afterglow_doom.c", "doom/autopilot.c", "doom/ap_nav.c"].map(Into::into));
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
