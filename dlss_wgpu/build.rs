use std::{env, path::PathBuf}; // 读取SDK位置并构造Cargo私有构建输出路径。
use std::{fs, path::Path}; // 复制允许的SR/RR运行库并清除生成目录中的非白名单NGX文件。

const WINDOWS_RUNTIME_DLLS: [&str; 2] = [
    "nvngx_dlss.dll",  // DLSS Super Resolution正式运行库。
    "nvngx_dlssd.dll", // DLSS Ray Reconstruction正式运行库。
]; // Frame Generation的nvngx_dlssg.dll不得进入运行时搜索目录。

/// 在Cargo私有输出中建立只含SR/RR的Windows NGX运行时目录。
fn install_windows_runtime_dlls(dlss_sdk: &Path, platform: &str, out_dir: &Path) -> PathBuf {
    let profile = if cfg!(feature = "debug_overlay") {
        "dev" // 显式debug_overlay构建继续使用SDK开发版运行库。
    } else {
        "rel" // 普通开发与正式候选都只使用SDK release运行库。
    }; // 完成当前crate特性到SDK目录的稳定映射。
    let source_dir = dlss_sdk
        .join("lib") // 进入SDK运行库根目录。
        .join(platform) // 使用目标平台的 NGX 运行库。
        .join(profile); // 选择与debug_overlay特性一致的dev或rel目录。
    let runtime_dir = out_dir.join("dlss_runtime"); // Cargo哈希输出目录由本次构建独占，可安全作为运行时白名单。
    fs::create_dir_all(&runtime_dir).unwrap_or_else(|error| {
        panic!(
            "无法创建DLSS运行时白名单目录{}：{error}",
            runtime_dir.display()
        )
    }); // 构建失败必须明确停止，不能回退到包含Frame Generation的完整SDK目录。

    for entry in fs::read_dir(&runtime_dir).unwrap_or_else(|error| {
        panic!(
            "无法审计DLSS运行时白名单目录{}：{error}",
            runtime_dir.display()
        )
    }) {
        let entry = entry.unwrap_or_else(|error| panic!("无法读取DLSS运行时目录项：{error}")); // 坏目录项不得绕过白名单审计。
        let file_name = entry.file_name(); // 冻结当前目录项名称供大小写无关判断。
        let file_name = file_name.to_string_lossy(); // NVIDIA官方ASCII文件名不会发生有损转换。
        let is_ngx_runtime = file_name.to_ascii_lowercase().starts_with("nvngx_"); // 只清理由本功能域所有的NGX运行库。
        let is_allowed = WINDOWS_RUNTIME_DLLS
            .iter() // 固定两项白名单不会形成规模相关工作。
            .any(|allowed| file_name.eq_ignore_ascii_case(allowed)); // Windows文件名按大小写无关语义核对。
        if is_ngx_runtime && !is_allowed && entry.path().is_file() {
            fs::remove_file(entry.path()).unwrap_or_else(|error| {
                panic!(
                    "无法移除非白名单NGX运行库{}：{error}",
                    entry.path().display()
                )
            }); // 旧构建残留的dlssg或其他nvngx文件不得继续被NGX扫描加载。
        } // 非NGX辅助文件不属于本函数清理范围。
    } // 完成生成目录中历史NGX文件的白名单收口。

    for file_name in WINDOWS_RUNTIME_DLLS {
        let source = source_dir.join(file_name); // 只从当前SDK固定profile读取本项白名单文件。
        let destination = runtime_dir.join(file_name); // 目标目录不暴露SDK中的任何相邻功能DLL。
        println!("cargo:rerun-if-changed={}", source.display()); // SDK运行库更新后必须重新复制并重编译路径。
        fs::copy(&source, &destination).unwrap_or_else(|error| {
            panic!(
                "复制DLSS运行库{}到{}失败：{error}",
                source.display(),
                destination.display()
            )
        }); // 缺少SR或RR任一文件都明确失败，不允许把整个SDK目录作为兼容回退。
    } // 完成两项正式运行库复制。

    runtime_dir // 返回将写入当前crate编译期环境的唯一允许搜索目录。
} // 完成Windows NGX运行时白名单准备。

fn main() {
    if cfg!(feature = "mock") {
        return;
    }

    // Get SDK paths
    println!("cargo:rerun-if-env-changed=DLSS_SDK"); // SDK根变更必须重新生成绑定和运行时白名单。
    println!("cargo:rerun-if-env-changed=VULKAN_SDK"); // Vulkan头文件根变更必须重新生成绑定。
    let dlss_sdk = PathBuf::from(
        env::var_os("DLSS_SDK")
            .expect("DLSS_SDK environment variable not set. Consult the dlss_wgpu readme."),
    ); // 保留非UTF-8路径能力并统一供链接、绑定与DLL复制使用。
    let vulkan_sdk =
        PathBuf::from(env::var_os("VULKAN_SDK").expect("VULKAN_SDK environment variable not set")); // Vulkan SDK只在构建期提供头文件。
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set")); // Cargo私有输出承载生成绑定与白名单DLL。

    // Determine platform info
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let platform = if target_os == "windows" {
        format!("Windows_{target_arch}")
    } else {
        format!("Linux_{target_arch}")
    };
    println!("cargo:rustc-env=DLSS_SDK_PLATFORM={platform}");

    // Link to needed libraries
    let mut lib_dir = dlss_sdk.join("lib").join(&platform);
    if target_os == "windows" && target_arch == "x86_64" {
        lib_dir.push("x64");
    }
    if !lib_dir.is_dir() {
        panic!("DLSS SDK at {} has no libraries for {platform}", dlss_sdk.display());
    }
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    if target_os == "windows" {
        #[cfg(not(target_feature = "crt-static"))]
        println!("cargo:rustc-link-lib=static=nvsdk_ngx_d");
        #[cfg(target_feature = "crt-static")]
        println!("cargo:rustc-link-lib=static=nvsdk_ngx_s");
        let runtime_dir = install_windows_runtime_dlls(&dlss_sdk, &platform, &out_dir);
        println!("cargo:rustc-env=DLSS_RUNTIME_DLL_DIR={}", runtime_dir.display());
    } else {
        println!("cargo:rustc-link-lib=static=nvsdk_ngx");
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=dl");
    }

    // Generate rust bindings
    #[cfg(not(target_os = "windows"))]
    let vulkan_sdk_include = "include";
    #[cfg(target_os = "windows")]
    let vulkan_sdk_include = "Include";
    bindgen::Builder::default()
        .header(format!("{}/src/wrapper.h", env!("CARGO_MANIFEST_DIR")))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .wrap_static_fns(true)
        .wrap_static_fns_path(out_dir.join("wrap_static_fns"))
        .clang_arg(format!("-I{}/include", dlss_sdk.display()))
        .clang_arg(format!("-I{}/{vulkan_sdk_include}", vulkan_sdk.display()))
        .allowlist_item(".*NGX.*")
        .blocklist_item("Vk.*")
        .blocklist_item("PFN_vk.*")
        .blocklist_item(".*Cuda.*")
        .blocklist_item(".*CUDA.*")
        .formatter(bindgen::Formatter::None) // 绑定由编译器消费，无需外部 Rust 格式化程序。
        .generate()
        .unwrap()
        .write_to_file(out_dir.join("bindings.rs"))
        .unwrap();

    // Generate and link a library for static inline functions
    cc::Build::new()
        .file(out_dir.join("wrap_static_fns.c"))
        .includes([
            format!("{}/include", dlss_sdk.display()),
            format!("{}/{vulkan_sdk_include}", vulkan_sdk.display()),
        ])
        .compile("wrap_static_fns");
}
