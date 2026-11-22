pub use gpui_agent_presentation::Language;
use std::path::Path;

pub fn load(path: &Path) -> Language { std::fs::read_to_string(path).map(|code| Language::from_code(&code)).unwrap_or_default() }

pub fn save(language: Language, path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, language.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_round_trips_and_unknown_values_fall_back_to_korean() {
        let path = std::env::temp_dir()
            .join(format!("kangnodle-language-{}", std::process::id()))
            .join("ui-language");
        assert_eq!(load(&path), Language::Korean);
        for language in [Language::English, Language::Korean] {
            save(language, &path).unwrap();
            assert_eq!(load(&path), language);
        }
        std::fs::write(&path, "unknown").unwrap();
        assert_eq!(load(&path), Language::Korean);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }
}
