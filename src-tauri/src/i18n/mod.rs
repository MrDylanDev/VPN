//! Internationalization (i18n) module.
//!
//! Loads translations from JSON files at startup and provides a `t!()` macro
//! for looking up localized strings. Falls back to English when a key is
//! missing in the selected locale.
//!
//! ## Usage
//!
//! ```ignore
//! use crate::i18n::I18n;
//!
//! let i18n = I18n::new("es").unwrap();
//! assert_eq!(i18n.t("button.connect"), "Conectar");
//! assert_eq!(i18n.t("nonexistent.key"), "nonexistent.key"); // fallback to key
//! ```

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Loaded translations for all supported locales.
pub struct I18n {
    locales: HashMap<String, HashMap<String, String>>,
    current: String,
}

impl I18n {
    /// Creates a new `I18n` instance, loading all translation files.
    ///
    /// The `default_locale` parameter sets the initial language.
    /// Supported locales are loaded from JSON files in the same directory
    /// as this module.
    pub fn new(default_locale: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut locales: HashMap<String, HashMap<String, String>> = HashMap::new();

        // Load all JSON translation files co-located with this module
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let i18n_dir = manifest_dir.join("src").join("i18n");

        if let Ok(entries) = fs::read_dir(&i18n_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "json") {
                    if let Some(locale) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(content) = fs::read_to_string(&path) {
                            if let Ok(translations) = Self::flatten_json(&content) {
                                locales.insert(locale.to_string(), translations);
                            }
                        }
                    }
                }
            }
        }

        // Ensure English fallback is always present
        if !locales.contains_key("en") {
            locales.insert("en".to_string(), HashMap::new());
        }

        let locale = if locales.contains_key(default_locale) {
            default_locale.to_string()
        } else {
            "en".to_string()
        };

        Ok(Self {
            locales,
            current: locale,
        })
    }

    /// Flattens a JSON object into a `HashMap<String, String>` of key paths.
    ///
    /// Nested keys are flattened with dot notation, e.g.:
    /// `{"button": {"connect": "Connect"}}` becomes `{"button.connect": "Connect"}`.
    fn flatten_json(json_str: &str) -> Result<HashMap<String, String>, serde_json::Error> {
        let value: Value = serde_json::from_str(json_str)?;
        let mut map = HashMap::new();
        Self::flatten_value(&value, String::new(), &mut map);
        Ok(map)
    }

    fn flatten_value(value: &Value, prefix: String, map: &mut HashMap<String, String>) {
        match value {
            Value::Object(obj) => {
                for (key, val) in obj {
                    let new_key = if prefix.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", prefix, key)
                    };
                    Self::flatten_value(val, new_key, map);
                }
            }
            Value::String(s) => {
                map.insert(prefix, s.clone());
            }
            _ => {}
        }
    }

    /// Returns the translated string for the given key in the current locale.
    ///
    /// Falls back to English if the key is missing in the current locale.
    /// Returns the key itself as a last resort.
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        // Try current locale
        if let Some(locale) = self.locales.get(&self.current) {
            if let Some(value) = locale.get(key) {
                return value;
            }
        }

        // Fall back to English
        if let Some(en) = self.locales.get("en") {
            if let Some(value) = en.get(key) {
                return value;
            }
        }

        // Last resort: return the key itself
        key
    }

    /// Sets the current locale. Returns an error if the locale is not loaded.
    pub fn set_locale(&mut self, locale: &str) -> Result<(), Box<dyn std::error::Error>> {
        if self.locales.contains_key(locale) {
            self.current = locale.to_string();
            Ok(())
        } else {
            Err(format!("Locale '{}' is not loaded", locale).into())
        }
    }

    /// Returns the current locale code (e.g., "en", "es").
    pub fn current_locale(&self) -> &str {
        &self.current
    }

    /// Returns a list of all available locale codes.
    pub fn available_locales(&self) -> Vec<String> {
        let mut locales: Vec<String> = self.locales.keys().cloned().collect();
        locales.sort();
        locales
    }
}

/// Macro for looking up localized strings at compile-time.
///
/// # Usage
///
/// ```ignore
/// let i18n: &I18n = get_i18n_state();
/// let label = t!(i18n, "button.connect");
/// ```
#[macro_export]
macro_rules! t {
    ($i18n:expr, $key:expr) => {
        $i18n.t($key)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_english_translation() {
        let i18n = I18n::new("en").unwrap();
        assert_eq!(i18n.t("button.connect"), "Connect");
        assert_eq!(i18n.t("app.name"), "WireGuard VPN Client");
    }

    #[test]
    fn test_spanish_translation() {
        let i18n = I18n::new("es").unwrap();
        assert_eq!(i18n.t("button.connect"), "Conectar");
        assert_eq!(i18n.t("app.name"), "Cliente VPN WireGuard");
    }

    #[test]
    fn test_missing_key_fallback() {
        let i18n = I18n::new("es").unwrap();
        // Falls back to key itself when neither locale has it
        assert_eq!(i18n.t("completely.missing.key"), "completely.missing.key");
    }

    #[test]
    fn test_available_locales() {
        let i18n = I18n::new("en").unwrap();
        let locales = i18n.available_locales();
        assert!(locales.contains(&"en".to_string()));
        assert!(locales.contains(&"es".to_string()));
    }

    #[test]
    fn test_default_to_en_for_unknown_locale() {
        let i18n = I18n::new("fr").unwrap();
        // Falls back to English if locale doesn't exist
        assert_eq!(i18n.current_locale(), "en");
    }

    #[test]
    fn test_set_locale() {
        let mut i18n = I18n::new("en").unwrap();
        assert_eq!(i18n.t("button.connect"), "Connect");
        i18n.set_locale("es").unwrap();
        assert_eq!(i18n.t("button.connect"), "Conectar");
    }

    #[test]
    fn test_macro() {
        let i18n = I18n::new("en").unwrap();
        assert_eq!(t!(i18n, "app.name"), "WireGuard VPN Client");
    }
}
