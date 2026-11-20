use crate::{config::Config, pick_best_android_ndk_toolchain, run_command};
use std::{fs, io::Write, path::PathBuf, process::Command};

// The prefix to add to all symbols
// Using crate name to avoid collisions with other projects
const PREFIX: &str = env!("CARGO_CRATE_NAME");

// Callback to add a `link_name` macro with the prefix to all generated bindings
#[derive(Debug)]
pub struct PrefixCallback;

impl bindgen::callbacks::ParseCallbacks for PrefixCallback {
    fn generated_link_name_override(
        &self,
        item_info: bindgen::callbacks::ItemInfo<'_>,
    ) -> Option<String> {
        Some(format!("{PREFIX}_{}", item_info.name))
    }
}

fn android_toolchain(config: &Config) -> PathBuf {
    let mut android_bin_path = config
        .env
        .android_ndk_home
        .clone()
        .expect("Please set ANDROID_NDK_HOME for Android build");
    android_bin_path.extend(["toolchains", "llvm", "prebuilt"]);
    android_bin_path.push(pick_best_android_ndk_toolchain(&android_bin_path).unwrap());
    android_bin_path.push("bin");
    android_bin_path
}

// Yapfast: LLVM's own tools for Windows targets, whose archives are COFF:
// the toolchain scripts/windows/bootstrap.ps1 installs sets `LLVM_HOME` and
// puts its `bin` on `PATH`. UNTESTED: no Windows build has run this yet.
fn llvm_tool(name: &str) -> PathBuf {
    match std::env::var_os("LLVM_HOME") {
        Some(home) => PathBuf::from(home).join("bin").join(format!("{name}.exe")),
        None => PathBuf::from(name),
    }
}

pub fn prefix_symbols(config: &Config) {
    // List static libraries to prefix symbols in
    // Yapfast: MSVC's are `ssl.lib` and `crypto.lib`, under the generator's
    // configuration folder when it makes one (`msvc_lib_subdir`).
    let windows = config.target_os == "windows";
    let names = if windows {
        ["ssl.lib", "crypto.lib"]
    } else {
        ["libssl.a", "libcrypto.a"]
    };
    let subdir = crate::msvc_lib_subdir(config);
    let static_libs: Vec<PathBuf> = [
        config.out_dir.join("build"),
        config.out_dir.join("build").join("ssl"),
        config.out_dir.join("build").join("crypto"),
    ]
    .iter()
    .flat_map(|dir| [Some(dir.clone()), subdir.map(|subdir| dir.join(subdir))])
    .flatten()
    .flat_map(|dir| names.into_iter().map(move |file| dir.join(file)))
    .filter(|p| p.exists())
    .collect();

    // Use `nm` to list symbols in these static libraries
    let nm = match &*config.target_os {
        "android" => android_toolchain(config).join("llvm-nm"),
        "windows" => llvm_tool("llvm-nm"),
        _ => PathBuf::from("nm"),
    };
    let out = run_command(Command::new(nm).args(&static_libs)).unwrap();
    // Yapfast: every global symbol the archives define is renamed, BoringSSL's
    // C++ ones (`_Z*` on ELF, `?*` on COFF) and weak objects (` V `, such as
    // its classes' typeinfo) included. Upstream kept names starting with `_`,
    // so all of `bssl::` stayed as it was, and LiveKit's libwebrtc, which
    // carries a BoringSSL of its own with the same C++ names, failed the
    // link with duplicate symbols (vendor/btls-sys/PATCHES.md). Only the
    // compiler's pointer to the C++ personality routine keeps its name: every
    // object in the program shares that one by design.
    let mut redefine_syms: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            [" T ", " D ", " B ", " C ", " R ", " W ", " V "]
                .iter()
                .any(|s| l.contains(s))
        })
        .filter_map(|l| l.split_whitespace().nth(2).map(|s| s.to_string()))
        .filter(|l| !l.starts_with("DW.ref."))
        .map(|l| format!("{l} {PREFIX}_{l}"))
        .collect();
    redefine_syms.sort();
    redefine_syms.dedup();

    let redefine_syms_path = config.out_dir.join("redefine_syms.txt");
    let mut f = fs::File::create(&redefine_syms_path).unwrap();
    for sym in &redefine_syms {
        writeln!(f, "{sym}").unwrap();
    }
    f.flush().unwrap();

    // Use `objcopy` to prefix symbols in these static libraries
    let objcopy = match &*config.target_os {
        "android" => android_toolchain(config).join("llvm-objcopy"),
        "windows" => llvm_tool("llvm-objcopy"),
        _ => PathBuf::from("objcopy"),
    };
    for static_lib in &static_libs {
        run_command(
            Command::new(&objcopy)
                .arg(format!("--redefine-syms={}", redefine_syms_path.display()))
                .arg(static_lib),
        )
        .unwrap();
    }
}
