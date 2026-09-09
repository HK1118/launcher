use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    if let Ok(json) = serde_json::to_string_pretty(apps) {
        let _ = std::fs::write(path, json);
    }
}

pub fn save_settings(settings: &Settings) {
    let path = get_data_path("settings.json");
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, json);
    }
}

/// 設定ファイルを安全に読み込みます。構文エラー等の破損時は上書きせず警告を返します。
pub fn load_settings() -> (Settings, Option<String>) {
    let path = get_data_path("settings.json");
    if !path.exists() {
        let defaults = Settings::default();
        if let Ok(json) = serde_json::to_string_pretty(&defaults) {
            let _ = std::fs::write(&path, json);
        }
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
