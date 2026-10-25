use alvr_localization::{CJK_FONT, Language};
use eframe::egui::{Context, FontData, FontDefinitions, FontFamily};
use std::{fs, sync::Arc};

const LANGUAGE_FILE: &str = "dashboard_language.txt";

pub fn initial_language() -> Language {
    load_language().unwrap_or_else(Language::detect)
}

pub fn window_title(language: Language) -> String {
    if language == Language::Chinese {
        format!("ALVR 控制面板（串流端 v{}）", *alvr_common::ALVR_VERSION)
    } else {
        format!("ALVR Dashboard (streamer v{})", *alvr_common::ALVR_VERSION)
    }
}

pub fn apply_language(language: Language, context: &Context) {
    alvr_localization::set_language(language);
    install_fonts(context, language);
}

fn install_fonts(context: &Context, language: Language) {
    let mut fonts = FontDefinitions::default();
    settings_font_data(&mut fonts);

    let proportional = fonts.families.get_mut(&FontFamily::Proportional).unwrap();
    if language == Language::Chinese {
        proportional.insert(0, "noto_sans_sc".to_owned());
    } else {
        proportional.push("noto_sans_sc".to_owned());
    }

    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .push("noto_sans_sc".to_owned());

    context.set_fonts(fonts);
}

fn settings_font_data(fonts: &mut FontDefinitions) {
    fonts.font_data.insert(
        "noto_sans_sc".to_owned(),
        Arc::new(FontData::from_static(CJK_FONT)),
    );
}

#[cfg(not(target_arch = "wasm32"))]
fn language_path() -> std::path::PathBuf {
    crate::get_filesystem_layout()
        .config_dir
        .join(LANGUAGE_FILE)
}

#[cfg(not(target_arch = "wasm32"))]
fn load_language() -> Option<Language> {
    fs::read_to_string(language_path())
        .ok()
        .and_then(|value| Language::from_str(&value))
}

#[cfg(target_arch = "wasm32")]
fn load_language() -> Option<Language> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_language(language: Language) {
    let path = language_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(path, language.as_str()).ok();
}

#[cfg(target_arch = "wasm32")]
pub fn save_language(_: Language) {}

#[cfg(test)]
mod tests {
    use super::window_title;
    use alvr_localization::Language;

    #[test]
    fn window_title_tracks_selected_language() {
        assert!(window_title(Language::Chinese).starts_with("ALVR 控制面板"));
        assert!(window_title(Language::English).starts_with("ALVR Dashboard"));
    }
}
