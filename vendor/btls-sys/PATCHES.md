# Anomi's btls-sys

This is btls-sys 0.5.6 from crates.io, the BoringSSL under wreq, under its
MIT licence (`LICENSE-MIT`; BoringSSL's own licences are in
`deps/boringssl/LICENSE`). It is the crate as published, `.cargo-ok` aside,
30 MB, almost all of it BoringSSL's sources, which the build compiles. The
root `Cargo.toml` patches it in through `[patch.crates-io]` and keeps it out
of the workspace (`exclude`). Cargo ignores the patch unless the name and
version stay crates.io's: moving wreq to another btls-sys means vendoring
that version and carrying these changes over. Each change is marked
`Yapfast:` in the source.

## Every symbol renamed

`build/prefix.rs` `prefix_symbols`. With the `prefix-symbols` feature, which
`crates/yapfast-http` turns on, btls-sys renames the global symbols its
`libssl.a` and `libcrypto.a` define to `build_script_main_*` with `nm` and
`objcopy`, and points its bindings at the new names. The prefix is
`env!("CARGO_CRATE_NAME")` read inside its build script, whose crate is
`build_script_main`.

Upstream skipped every name starting with `_`, which on Linux is every C++
symbol: all of `bssl::`, `evp_pkey_ctx_st::*`, `ssl_credential_st::*`,
`FIPS_service_indicator_*()` and the typeinfo of BoringSSL's classes. LiveKit's
`libwebrtc.a` carries a BoringSSL of its own whose C++ symbols keep the same
names, so every binary linking both (`anomi`, `yapfast`, yapfast-discord's
programs and tests) failed in rust-lld with `duplicate symbol`. The archives
are bundled into btls-sys's rlib, so renaming them after the build does
nothing; the build script has to.

Now every global symbol the archives define is renamed, weak objects (`V`)
included, but `DW.ref.__gxx_personality_v0`, the compiler's pointer to the
C++ personality routine, which every object in a program shares by design.
Measured on Linux with BoringSSL as btls-sys 0.5.6 builds it: 4,307 global
symbols defined, all renamed but that one; 434 of them C++ (`_Z*`), of which
413 libwebrtc.a defines too. Built from a clean `target/fast/build`, it links,
and `nm target/fast/anomi` shows wreq's 1,639 symbols prefixed (277 of them
C++) beside libwebrtc's 886 unprefixed `bssl::` ones.

To carry over: in `prefix_symbols`, the type filter takes `" V "` as well,
and the name filter `!l.starts_with("_")` becomes
`!l.starts_with("DW.ref.")`.

## Windows, untested

`build/prefix.rs` `prefix_symbols` and `llvm_tool`, `build/main.rs` `run`.
Upstream skipped renaming for Windows targets but still pointed the bindings
at the prefixed names, so the feature couldn't be on there, and wreq's
BoringSSL went unprefixed beside `webrtc.lib`'s. Now Windows targets are
renamed too: the archives are `ssl.lib` and `crypto.lib`, also looked for
under the MSVC generator's configuration folder (`msvc_lib_subdir`), and the
tools are `llvm-nm` and `llvm-objcopy` from `LLVM_HOME/bin` (set by
`scripts/windows/bootstrap.ps1`, which now requires both) or `PATH`. COFF
names C++ symbols `?...`, which the same rule takes. No Windows build has run
this yet; if `llvm-objcopy` refuses the archives the build stops in
btls-sys's build script, and if a name is missed the link reports it.
macOS and iOS stay skipped, as upstream.

## Bindings' warnings

`src/lib.rs`, `mod generated`: allows `unnecessary_transmutes`. bindgen 0.72
writes bitfield accessors that Rust 1.98 warns about; a registry crate's
warnings are capped, a path dependency's are printed on every build.

## Windows linker response files

`build/main.rs`, `emit_link_directives`: keep the base library directory as
it is when the subdirectory is empty. `Path::join("")` adds a trailing
backslash on Windows. With Ninja, there is no MSVC configuration subfolder
to append, so that backslash reached `cargo:rustc-link-search`. On long
link commands, Rust 1.98.1 puts the arguments into an MSVC response file
without escaping the trailing backslash before the closing quote. That
merges subsequent options into the library path, including `/OUT` and
`/SUBSYSTEM`: linking reports success but writes `symbols.exe` in the
working directory instead of the requested GUI executable.

Verified with LLVM 21.1.7 using a UTF-16 response file: the trailing
backslash loses the output name and GUI subsystem; the same directory
without it preserves both. Keep the empty-subdirectory case when updating
this vendored crate.
