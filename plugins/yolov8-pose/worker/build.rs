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
    let vendor = plugin.join("vendor");
    let yolo = vendor.join("yolov8_pose");
    let utils = vendor.join("utils");
    let rknn_include = deps.join("rknpu2/include");
    let rga_include = deps.join("librga/include");
    let stb_include = vendor.join("stb_image");
    let jpeg_include = deps.join("jpeg_turbo/include");

    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .files([
            "native/bridge.cc".into(),
            yolo.join("postprocess.cc"),
            yolo.join("rknpu2/yolov8-pose.cc"),
        ])
        .includes([&yolo, &utils, &rknn_include, &rga_include])
        .flag_if_supported("-Wno-write-strings")
        .compile("rkserve_yolov8_pose_native");

    // Static link order matters: the native engine references symbols from the utility archive.
    cc::Build::new()
        .files([utils.join("file_utils.c"), utils.join("image_utils.c")])
        .includes([&utils, &rga_include, &stb_include, &jpeg_include])
        .flag_if_supported("-Wno-error=incompatible-pointer-types")
        .flag_if_supported("-Wno-error=discarded-qualifiers")
        .compile("rkserve_yolov8_pose_utils");

    for lib in ["rknpu2/lib", "librga/lib", "jpeg_turbo/lib"] {
        println!("cargo:rustc-link-search=native={}", deps.join(lib).display());
    }
    println!("cargo:rustc-link-lib=dylib=rknnrt");
    println!("cargo:rustc-link-lib=dylib=rga");
    println!("cargo:rustc-link-lib=static=turbojpeg");
    println!("cargo:rustc-link-lib=dylib=stdc++");
    println!("cargo:rustc-link-lib=dylib=m");
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../lib");
    println!("cargo:rerun-if-changed=native/bridge.cc");
    println!("cargo:rerun-if-env-changed=RKSERVE_PLUGIN_DEPS");
    for source in [
        yolo.join("postprocess.cc"),
        yolo.join("postprocess.h"),
        yolo.join("yolov8-pose.h"),
        yolo.join("rknpu2/yolov8-pose.cc"),
        utils.join("common.h"),
        utils.join("file_utils.c"),
        utils.join("file_utils.h"),
        utils.join("image_utils.c"),
        utils.join("image_utils.h"),
    ] {
        println!("cargo:rerun-if-changed={}", source.display());
    }
}
