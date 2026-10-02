use std::{env, path::PathBuf};

fn main() {
    let plugin = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("missing manifest directory"))
        .parent()
        .expect("worker must be inside its plugin directory")
        .to_path_buf();
    let plugins_root = plugin.parent().expect("plugin must be inside the plugins directory");
    // Third-party headers and libraries fetched by plugins/tools/fetch-deps.sh.
    let deps = env::var_os("RKSERVE_PLUGIN_DEPS")
        .map(PathBuf::from)
        .unwrap_or_else(|| plugins_root.join(".deps"));

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .file("native/bridge.cc")
        .include(deps.join("rknpu2/include"))
        .include(deps.join("onnxruntime/include"))
        .flag_if_supported("-Wno-unused-parameter")
        .compile("rkserve_matcha_tts_native");

    println!("cargo:rustc-link-search=native={}", deps.join("rknpu2/lib").display());
    println!("cargo:rustc-link-lib=dylib=rknnrt");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=m");
    println!("cargo:rustc-link-lib=dylib=dl");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
    println!("cargo:rerun-if-changed=native/bridge.cc");
    println!("cargo:rerun-if-env-changed=RKSERVE_PLUGIN_DEPS");
}
