use crate::nvsdk_ngx::*;
use std::{
    env,
    ffi::{CString, OsStr},
    ptr,
};
#[cfg(not(target_os = "windows"))] // 非Windows平台继续使用上游SDK共享库目录发现逻辑。
use std::{env::var, ffi::OsString}; // Linux路径仍由运行时DLSS_SDK环境变量构造。
#[cfg(target_os = "windows")]
// Windows必须拒绝整个SDK目录，避免误加载未使用的Frame Generation。
use std::{
    fs,
    path::{Path, PathBuf},
}; // 审计并选择可执行文件旁或构建期生成的SR/RR白名单目录。
use uuid::Uuid;

pub fn with_feature_info<F, T>(project_id: Uuid, feature_id: NVSDK_NGX_Feature, callback: F) -> T
where
    F: FnOnce(&NVSDK_NGX_FeatureDiscoveryInfo) -> T,
{
    let project_id = CString::new(project_id.to_string()).unwrap();
    let engine_version = CString::new(env!("CARGO_PKG_VERSION")).unwrap();
    let data_path = os_str_to_wchar(env::temp_dir().as_os_str());

    let shared_library_paths = get_shared_library_paths();
    let shared_library_path_pointers = shared_library_paths
        .iter()
        .map(Vec::as_ptr)
        .collect::<Vec<_>>();

    let feature_info_common = NVSDK_NGX_FeatureCommonInfo {
        PathListInfo: NVSDK_NGX_PathListInfo {
            Path: shared_library_path_pointers.as_ptr(),
            Length: shared_library_paths.len() as u32,
        },
        InternalData: ptr::null_mut(),
        // TODO: Allow configuring logging
        LoggingInfo: NVSDK_NGX_LoggingInfo {
            LoggingCallback: None,
            MinimumLoggingLevel: NVSDK_NGX_Logging_Level_NVSDK_NGX_LOGGING_LEVEL_OFF,
            DisableOtherLoggingSinks: false,
        },
    };

    let feature_info = NVSDK_NGX_FeatureDiscoveryInfo {
        SDKVersion: NVSDK_NGX_Version_NVSDK_NGX_Version_API,
        FeatureID: feature_id,
        Identifier: NVSDK_NGX_Application_Identifier {
            IdentifierType: NVSDK_NGX_Application_Identifier_Type_NVSDK_NGX_Application_Identifier_Type_Project_Id,
            v: NVSDK_NGX_Application_Identifier_v {
                ProjectDesc: NVSDK_NGX_ProjectIdDescription {
                    ProjectId: project_id.as_ptr(),
                    EngineType: NVSDK_NGX_EngineType_NVSDK_NGX_ENGINE_TYPE_CUSTOM,
                    EngineVersion: engine_version.as_ptr(),
                },
            },
        },
        ApplicationDataPath: data_path.as_ptr(),
        FeatureInfo: &feature_info_common,
    };

    (callback)(&feature_info)
}

#[cfg(target_os = "windows")] // Windows运行时只允许项目正式声明的两个NGX功能DLL。
const WINDOWS_RUNTIME_DLLS: [&str; 2] = [
    "nvngx_dlss.dll",  // DLSS Super Resolution运行库。
    "nvngx_dlssd.dll", // DLSS Ray Reconstruction运行库。
]; // nvngx_dlssg.dll及其他nvngx运行库均不属于当前项目。

/// 返回目录内全部普通`nvngx_*`文件的大小写规范化名称。
#[cfg(target_os = "windows")] // 目录审计只在Windows DLL加载路径上需要。
fn windows_ngx_runtime_file_names(directory: &Path) -> Option<Vec<String>> {
    let entries = fs::read_dir(directory).ok()?; // 无法完整读取的目录不能被当作已验证白名单。
    let mut names = Vec::new(); // 正式目录固定只有两项，微小分配只发生在NGX初始化阶段。
    for entry in entries {
        let entry = entry.ok()?; // 任一目录项读取失败都会使整目录失去采用资格。
        if !entry.path().is_file() {
            continue; // 只跳过目录；指向DLL的符号链接仍必须进入白名单审计。
        } // 完成普通文件过滤。
        let name = entry.file_name(); // 冻结当前目录项名称，避免临时值生命周期跨表达式。
        let name = name.to_string_lossy().to_ascii_lowercase(); // NVIDIA官方ASCII名称按Windows大小写无关语义规范化。
        if name.starts_with("nvngx_") {
            names.push(name); // 只收集会扩大NGX功能面的运行库文件。
        } // 普通游戏DLL不属于本白名单协议。
    } // 完成一次启动期小型目录审计。
    names.sort(); // 稳定顺序便于与固定白名单逐项比较和单测。
    Some(names) // 返回完整可审计名称集合。
} // 完成Windows NGX目录枚举。

/// 判断一个目录是否精确包含SR/RR且没有任何额外NGX运行库。
#[cfg(target_os = "windows")] // Windows专用白名单判定不影响Linux上游行为。
fn windows_runtime_directory_is_approved(directory: &Path) -> bool {
    let Some(names) = windows_ngx_runtime_file_names(directory) else {
        return false; // 不存在、不可读或含坏目录项时拒绝该候选路径。
    }; // 取得当前目录全部NGX文件名。
    names.len() == WINDOWS_RUNTIME_DLLS.len() // 数量先锁定为SR/RR两项。
        && names
            .iter() // 固定两项比较不形成规模相关启动工作。
            .zip(WINDOWS_RUNTIME_DLLS) // 排序后的实际名称与排序后的正式白名单配对。
            .all(|(actual, expected)| actual == expected) // 任一dlssg或其他NGX文件都会拒绝整个目录。
} // 完成Windows运行时目录白名单判定。

/// 优先选择交付包可执行文件旁的白名单，开发运行回退Cargo私有构建目录。
#[cfg(target_os = "windows")] // Windows需要在可搬运发布与本地开发之间选择安全路径。
fn select_windows_runtime_directory(
    executable_directory: Option<&Path>, // 正式发布由可执行文件同目录携带两项白名单DLL。
    build_directory: &Path,              // 普通cargo run使用build.rs生成的隔离目录。
) -> Option<PathBuf> {
    if let Some(directory) = executable_directory
        && windows_runtime_directory_is_approved(directory)
    {
        return Some(directory.to_path_buf()); // 完整发布包不依赖开发机Cargo target路径。
    } // 可执行文件目录缺失、不可读或混入额外NGX运行库时不得采用。
    windows_runtime_directory_is_approved(build_directory).then(|| build_directory.to_path_buf()) // 开发构建只暴露build.rs复制的SR/RR目录。
} // 完成Windows运行时目录选择。

/// 获取现有精确 SR/RR 白名单目录，供同进程 Streamline 首次 NGX 初始化复用。
#[cfg(target_os = "windows")] // 仅公开已经实施白名单协议的平台。
pub fn approved_windows_runtime_directory() -> Option<PathBuf> { // 不增加搜索目录，也不暴露整个外部 SDK。
    let executable_directory = env::current_exe().ok().and_then(|executable| { // 优先遵循现有交付包位置。
        executable.parent().map(Path::to_path_buf) // 返回当前程序所在目录。
    }); // 不猜其他磁盘路径。
    let build_directory = Path::new(env!("DLSS_RUNTIME_DLL_DIR")); // 既有构建脚本生成的两项私有目录。
    select_windows_runtime_directory(executable_directory.as_deref(), build_directory) // 复用唯一SR/RR完整目录审计，拒绝混入其他NGX文件。
} // 公共目录访问入口结束。

#[cfg(target_os = "windows")] // Windows不再把整个DLSS_SDK的rel目录交给NGX扫描。
fn get_shared_library_paths() -> Vec<Vec<wchar_t>> { // SR初始化与Streamline共享同一已批准路径。
    approved_windows_runtime_directory() // 选择逻辑只维护一处。
        .into_iter() // 无合法目录时不扩大搜索范围。
        .map(|directory| os_str_to_wchar(directory.as_os_str())) // NGX C API要求以NUL结尾的宽字符。
        .collect() // 当前协议最多一个明确白名单目录。
} // Windows路径构造结束。

#[cfg(not(target_os = "windows"))] // Linux保持上游按SDK平台和profile发现共享库的行为。
fn get_shared_library_paths() -> Vec<Vec<wchar_t>> {
    let mut shared_library_paths = vec![];

    // Set by build.rs
    let platform = env!("DLSS_SDK_PLATFORM");

    #[cfg(feature = "debug_overlay")]
    let profile = "dev";
    #[cfg(not(feature = "debug_overlay"))]
    let profile = "rel";

    // Look in $DLSS_SDK if set
    let sdk_path = var("DLSS_SDK").map(|sdk| format!("{sdk}/lib/{platform}/{profile}"));
    if let Ok(sdk_path) = sdk_path.as_ref() {
        shared_library_paths.push(os_str_to_wchar(&OsString::from(sdk_path)));
    }

    shared_library_paths
}

#[cfg(all(test, target_os = "windows"))] // 回归测试只验证本次Windows DLL加载边界。
mod tests {
    use super::*; // 复用生产白名单、目录审计与选择函数。

    #[test] // 开发目录必须物理排除Frame Generation，不能只在Rust功能枚举中不调用它。
    fn generated_runtime_directory_contains_only_sr_and_rr() {
        let directory = Path::new(env!("DLSS_RUNTIME_DLL_DIR")); // 读取本次build.rs实际生成路径。
        assert!(windows_runtime_directory_is_approved(directory)); // 生产采用门必须接受当前生成结果。
        assert_eq!(
            windows_ngx_runtime_file_names(directory).expect("生成目录必须可完整读取"),
            vec!["nvngx_dlss.dll", "nvngx_dlssd.dll"],
        ); // 精确锁定没有nvngx_dlssg.dll或其他隐藏NGX运行库。
    } // 完成构建期运行目录白名单回归。

    #[test] // 可执行文件旁未携带正式DLL时，cargo run必须使用隔离构建目录而非整个SDK。
    fn development_runtime_selects_the_generated_whitelist_directory() {
        let build_directory = Path::new(env!("DLSS_RUNTIME_DLL_DIR")); // 使用真实构建产物避免伪造文件系统状态。
        let selected = select_windows_runtime_directory(None, build_directory)
            .expect("生成的SR/RR白名单目录必须可采用"); // 模拟target/debug旁没有交付DLL的普通开发运行。
        assert_eq!(selected, build_directory); // 路径不得回退DLSS_SDK/lib/Windows_x86_64/rel。
    } // 完成开发运行路径回归。
} // 结束Windows NGX运行时白名单测试。

#[cfg(target_os = "windows")]
fn os_str_to_wchar(s: &OsStr) -> Vec<wchar_t> {
    use std::os::windows::ffi::OsStrExt;

    s.encode_wide().chain([0]).map(|c| c as wchar_t).collect()
}

#[cfg(not(target_os = "windows"))]
fn os_str_to_wchar(s: &OsStr) -> Vec<wchar_t> {
    s.to_str()
        .unwrap_or("")
        .chars()
        .chain([0 as char])
        .map(|c| c as wchar_t)
        .collect()
}
