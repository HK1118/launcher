use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct SavedApp {
    pub name: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    pub confirm_on_delete: bool,
    #[serde(default, alias = "show_edit_buttons")]
    pub edit_mode: bool,
    pub app_name_font_size: f32,
    pub app_path_font_size: f32,
    #[serde(default = "default_cooldown_secs")]
    pub launch_cooldown_secs: u64,
}

fn default_cooldown_secs() -> u64 {
    8
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            confirm_on_delete: true,
            edit_mode: false,
            app_name_font_size: 15.0,
            app_path_font_size: 12.0,
            launch_cooldown_secs: default_cooldown_secs(),
        }
    }
}

pub fn get_data_path(file_name: &str) -> PathBuf {
    #[cfg(debug_assertions)]
    {
        PathBuf::from(file_name)
    }
    #[cfg(not(debug_assertions))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|parent| parent.join(file_name)))
            .unwrap_or_else(|| PathBuf::from(file_name))
    }
}

/// 一時ファイルに書き込んでからリネームすることで、破損を防ぐアトミック保存
fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp_path = path.with_extension("tmp");

    {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }

    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

pub fn load_apps() -> Result<Vec<SavedApp>, String> {
    let path = get_data_path("apps.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("apps.json の読み込み失敗: {}", e))?;
    serde_json::from_str(&content).map_err(|e| format!("apps.json 構文エラー ({})", e))
}

pub fn save_apps(apps: &[SavedApp]) {
    let path = get_data_path("apps.json");
    let _ = atomic_write_json(&path, &apps);
}

pub fn save_settings(settings: &Settings) {
    let path = get_data_path("settings.json");
    let _ = atomic_write_json(&path, settings);
}

/// 設定ファイルを安全に読み込みます。構文エラー等の破損時は上書きせず警告を返します。
pub fn load_settings() -> (Settings, Option<String>) {
    let path = get_data_path("settings.json");
    if !path.exists() {
        let defaults = Settings::default();
        let _ = atomic_write_json(&path, &defaults);
        return (defaults, None);
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<Settings>(&content) {
            Ok(settings) => (settings, None),
            Err(e) => (
                Settings::default(),
                Some(format!(
                    "settings.json 構文エラー（デフォルトで動作）: {}",
                    e
                )),
            ),
        },
        Err(e) => (
            Settings::default(),
            Some(format!("settings.json 読込エラー: {}", e)),
        ),
    }
}
