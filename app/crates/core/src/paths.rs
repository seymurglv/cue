//! CUE directories: `%LOCALAPPDATA%\CUE` (last dictation, log, usage.csv, bench) and `%APPDATA%\CUE` (settings).
//! Paths are built by pure functions from a base directory; only `local_app_data` and `app_data` read
//! environment variables.
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `<base>\CUE`, where `base` is the value of `%LOCALAPPDATA%`.
pub fn local_dir(local_app_data: &Path) -> PathBuf {
    local_app_data.join("CUE")
}

/// `<base>\CUE\usage.csv`.
pub fn usage_csv(local_app_data: &Path) -> PathBuf {
    local_dir(local_app_data).join("usage.csv")
}

/// `<base>\CUE\bench`: bench recordings (`recordings\`) and its runs (`runs\`), kept outside the repository.
pub fn bench_dir(local_app_data: &Path) -> PathBuf {
    local_dir(local_app_data).join("bench")
}

/// `%LOCALAPPDATA%`. `None` if the variable is missing, empty or a relative path
/// (otherwise CUE files would land in the process's current directory).
pub fn local_app_data() -> Option<PathBuf> {
    base_from(std::env::var_os("LOCALAPPDATA"))
}

/// `<base>\CUE\settings.json`, where `base` is the value of `%APPDATA%`.
pub fn settings_json(app_data: &Path) -> PathBuf {
    app_data.join("CUE").join("settings.json")
}

/// `%APPDATA%`, by the same rule as [`local_app_data`].
pub fn app_data() -> Option<PathBuf> {
    base_from(std::env::var_os("APPDATA"))
}

fn base_from(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_are_built_from_the_base() {
        let base = std::env::temp_dir();
        let dir = local_dir(&base);
        assert_eq!(dir.parent(), Some(base.as_path()));
        assert_eq!(dir.file_name().unwrap(), "CUE");
        let csv = usage_csv(&base);
        assert_eq!(csv.parent(), Some(dir.as_path()));
        assert_eq!(csv.file_name().unwrap(), "usage.csv");
        let bench = bench_dir(&base);
        assert_eq!(bench.parent(), Some(dir.as_path()));
        assert_eq!(bench.file_name().unwrap(), "bench");
        let settings = settings_json(&base);
        assert_eq!(settings.parent(), Some(base.join("CUE").as_path()));
        assert_eq!(settings.file_name().unwrap(), "settings.json");
    }

    #[cfg(windows)]
    #[test]
    fn windows_layout_matches_expected_paths() {
        let base = Path::new(r"C:\Users\u\AppData\Local");
        assert_eq!(local_dir(base), PathBuf::from(r"C:\Users\u\AppData\Local\CUE"));
        assert_eq!(usage_csv(base), PathBuf::from(r"C:\Users\u\AppData\Local\CUE\usage.csv"));
        assert_eq!(bench_dir(base), PathBuf::from(r"C:\Users\u\AppData\Local\CUE\bench"));
        let roaming = Path::new(r"C:\Users\u\AppData\Roaming");
        assert_eq!(settings_json(roaming), PathBuf::from(r"C:\Users\u\AppData\Roaming\CUE\settings.json"));
    }

    #[test]
    fn base_must_be_present_non_empty_and_absolute() {
        assert_eq!(base_from(None), None);
        assert_eq!(base_from(Some(OsString::new())), None);
        assert_eq!(base_from(Some("relative".into())), None, "a relative path would write into the current directory");
        let abs = std::env::temp_dir();
        assert_eq!(base_from(Some(abs.clone().into_os_string())), Some(abs));
    }
}
