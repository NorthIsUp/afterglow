//! A no-op unless `--features doom`, `micropolis`, `mac` or `gba`, so a
//! plain `cargo build` needs no C or C++ compiler.
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
    #[cfg(feature = "mac")]
    mac();
    #[cfg(feature = "gba")]
    gba();
}

/// mGBA's GBA core (MPL-2.0), `gba/afterglow_gba.c` (its C API and crash
/// boundary) and `gba/blip_silent.c` (no audio), as one static library.
/// `gba/sources.txt` is what upstream's cmake builds for `MINIMAL_CORE` with
/// only `M_CORE_GBA`, without threads (`core/thread.c` out,
/// `DISABLE_THREADING`): the saver has its own thread.
#[cfg(feature = "gba")]
fn gba() {
    println!("cargo::rerun-if-changed=gba");
    let mut sources: Vec<std::path::PathBuf> = std::fs::read_to_string("gba/sources.txt")
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| format!("gba/mgba/src/{l}").into())
        .collect();
    sources.extend(["gba/afterglow_gba.c", "gba/blip_silent.c"].map(Into::into));
    cc::Build::new()
        .files(&sources)
        .include("gba/mgba/include")
        .include("gba/mgba/src")
        .std("c11")
        .define("M_CORE_GBA", None)
        .define("MINIMAL_CORE", None)
        .define("BUILD_STATIC", None)
        .define("DISABLE_THREADING", None)
        .define("HAVE_STRDUP", None)
        .define("HAVE_STRNDUP", None)
        .define("HAVE_STRLCPY", None)
        .define("HAVE_LOCALE", None)
        .define("HAVE_LOCALTIME_R", None)
        .define("_GNU_SOURCE", None)
        .define("abort", "agb_abort")
        .opt_level(3)
        .warnings(false)
        .flag_if_supported("-w")
        .compile("mgba");
    // The config code asks CoreFoundation for the app bundle on macOS.
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|o| o == "macos") {
        println!("cargo::rustc-link-lib=framework=CoreFoundation");
    }
}

/// Mini vMac's Mac Plus build and `mac/afterglow_mac.c`, its headless
/// platform layer, as one static library. Only `mac-engine` names it (its
/// `#[link]`); cc's own link line would put it in every binary.
#[cfg(feature = "mac")]
fn mac() {
    println!("cargo::rerun-if-changed=mac");
    let mut sources: Vec<std::path::PathBuf> = [
        "GLOBGLUE", "IWMEMDEV", "KBRDEMDV", "M68KITAB", "MINEM68K", "MOUSEMDV", "PROGMAIN",
        "ROMEMDEV", "RTCEMDEV", "SCCEMDEV", "SCRNEMDV", "SCSIEMDV", "SONYEMDV", "VIAEMDEV",
    ]
    .iter()
    .map(|f| format!("mac/minivmac/{f}.c").into())
    .collect();
    sources.push("mac/afterglow_mac.c".into());
    cc::Build::new()
        .files(&sources)
        .include("mac/minivmac/cfg")
        .include("mac/minivmac")
        .opt_level(2)
        .warnings(false)
        .flag_if_supported("-w")
        .cargo_metadata(false)
        .compile("minivmac");
    let out = std::env::var("OUT_DIR").unwrap();
    println!("cargo::rustc-link-search=native={out}");
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
