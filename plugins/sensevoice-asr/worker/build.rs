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
    let native = plugin.join("worker/native");
    let fbank = plugins_root.join("vendor/kaldi-native-fbank");

    let mut native_build = cc::Build::new();
    native_build
        .cpp(true)
        .std("c++17")
        .pic(true)
        .file(native.join("bridge.cc"))
        .files([
            fbank.join("src/feature-fbank.cc"),
            fbank.join("src/feature-functions.cc"),
            fbank.join("src/feature-mfcc.cc"),
            fbank.join("src/feature-window.cc"),
            fbank.join("src/fftsg.cc"),
            fbank.join("src/kaldi-math.cc"),
            fbank.join("src/mel-computations.cc"),
            fbank.join("src/online-feature.cc"),
            fbank.join("src/rfft.cc"),
            fbank.join("src/whisper-feature.cc"),
        ])
        .includes([deps.join("rknpu2/include"), fbank.join("include")])
        .warnings(true)
        .compile("rkserve_sensevoice_asr_native");

    println!("cargo:rustc-link-search=native={}", deps.join("rknpu2/lib").display());
    println!("cargo:rustc-link-lib=dylib=rknnrt");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=m");
    println!("cargo:rustc-link-lib=dylib=pthread");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
    println!("cargo:rerun-if-changed={}", native.join("bridge.cc").display());
    println!("cargo:rerun-if-env-changed=RKSERVE_PLUGIN_DEPS");
    println!("cargo:rerun-if-changed={}", fbank.join("src").display());
    println!("cargo:rerun-if-changed={}", fbank.join("include").display());
}
