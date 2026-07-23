use anyhow::Result;

const PREF_GEMINI_API_KEY: &str = "voice.gemini_api_key";
const PREF_GEMINI_MODEL: &str = "voice.gemini_model";
const PREF_GEMINI_VOICE: &str = "voice.gemini_voice";
const DEFAULT_GEMINI_MODEL: &str = "gemini-3.1-flash-live-preview";
const LEGACY_BATCH_MODEL: &str = "gemini-3.5-flash";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GeminiVoiceOption {
    pub name: &'static str,
    pub style: &'static str,
}

pub(crate) const GEMINI_VOICE_OPTIONS: &[GeminiVoiceOption] = &[
    GeminiVoiceOption {
        name: "Zephyr",
        style: "Brillante",
    },
    GeminiVoiceOption {
        name: "Puck",
        style: "Vivace",
    },
    GeminiVoiceOption {
        name: "Charon",
        style: "Informativa",
    },
    GeminiVoiceOption {
        name: "Kore",
        style: "Decisa",
    },
    GeminiVoiceOption {
        name: "Fenrir",
        style: "Energica",
    },
    GeminiVoiceOption {
        name: "Leda",
        style: "Giovane",
    },
    GeminiVoiceOption {
        name: "Orus",
        style: "Decisa",
    },
    GeminiVoiceOption {
        name: "Aoede",
        style: "Leggera",
    },
    GeminiVoiceOption {
        name: "Callirrhoe",
        style: "Rilassata",
    },
    GeminiVoiceOption {
        name: "Autonoe",
        style: "Brillante",
    },
    GeminiVoiceOption {
        name: "Enceladus",
        style: "Sussurrata",
    },
    GeminiVoiceOption {
        name: "Iapetus",
        style: "Chiara",
    },
    GeminiVoiceOption {
        name: "Umbriel",
        style: "Rilassata",
    },
    GeminiVoiceOption {
        name: "Algieba",
        style: "Morbida",
    },
    GeminiVoiceOption {
        name: "Despina",
        style: "Morbida",
    },
    GeminiVoiceOption {
        name: "Erinome",
        style: "Chiara",
    },
    GeminiVoiceOption {
        name: "Algenib",
        style: "Roca",
    },
    GeminiVoiceOption {
        name: "Rasalgethi",
        style: "Informativa",
    },
    GeminiVoiceOption {
        name: "Laomedeia",
        style: "Vivace",
    },
    GeminiVoiceOption {
        name: "Achernar",
        style: "Soffice",
    },
    GeminiVoiceOption {
        name: "Alnilam",
        style: "Decisa",
    },
    GeminiVoiceOption {
        name: "Schedar",
        style: "Equilibrata",
    },
    GeminiVoiceOption {
        name: "Gacrux",
        style: "Matura",
    },
    GeminiVoiceOption {
        name: "Pulcherrima",
        style: "Diretta",
    },
    GeminiVoiceOption {
        name: "Achird",
        style: "Amichevole",
    },
    GeminiVoiceOption {
        name: "Zubenelgenubi",
        style: "Casual",
    },
    GeminiVoiceOption {
        name: "Vindemiatrix",
        style: "Gentile",
    },
    GeminiVoiceOption {
        name: "Sadachbia",
        style: "Animata",
    },
    GeminiVoiceOption {
        name: "Sadaltager",
        style: "Competente",
    },
    GeminiVoiceOption {
        name: "Sulafat",
        style: "Calda",
    },
];

pub(crate) fn load_gemini_api_key() -> Option<String> {
    let db_path = pax_db::Database::default_path();
    let from_db = pax_db::Database::open(&db_path)
        .ok()
        .and_then(|db| load_gemini_api_key_from_db(&db));
    from_db.or_else(load_gemini_api_key_from_env)
}

pub(crate) fn save_gemini_api_key(value: &str) {
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return;
    };
    let _ = save_gemini_api_key_to_db(&db, value);
}

pub(crate) fn load_gemini_model() -> String {
    let db_path = pax_db::Database::default_path();
    pax_db::Database::open(&db_path)
        .ok()
        .and_then(|db| load_gemini_model_from_db(&db))
        .map(migrate_legacy_model)
        .or_else(load_gemini_model_from_env)
        .unwrap_or_else(|| DEFAULT_GEMINI_MODEL.to_string())
}

pub(crate) fn save_gemini_model(value: &str) {
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return;
    };
    let _ = save_gemini_model_to_db(&db, value);
}

pub(crate) fn load_gemini_voice() -> Option<String> {
    let db_path = pax_db::Database::default_path();
    let from_db = pax_db::Database::open(&db_path)
        .ok()
        .and_then(|db| load_gemini_voice_from_db(&db));
    from_db.or_else(load_gemini_voice_from_env)
}

pub(crate) fn save_gemini_voice(value: &str) {
    let db_path = pax_db::Database::default_path();
    let Ok(db) = pax_db::Database::open(&db_path) else {
        return;
    };
    let _ = save_gemini_voice_to_db(&db, value);
}

pub(crate) fn gemini_voice_labels() -> Vec<String> {
    std::iter::once("Automatica (Gemini)".to_string())
        .chain(
            GEMINI_VOICE_OPTIONS
                .iter()
                .map(|voice| format!("{} - {}", voice.name, voice.style)),
        )
        .collect()
}

pub(crate) fn gemini_voice_index(voice: Option<&str>) -> u32 {
    voice
        .and_then(|voice| {
            GEMINI_VOICE_OPTIONS
                .iter()
                .position(|option| option.name.eq_ignore_ascii_case(voice))
        })
        .map(|index| index as u32 + 1)
        .unwrap_or(0)
}

pub(crate) fn gemini_voice_at_index(index: u32) -> Option<&'static str> {
    index
        .checked_sub(1)
        .and_then(|index| GEMINI_VOICE_OPTIONS.get(index as usize))
        .map(|voice| voice.name)
}

pub(crate) fn gemini_voice_selection_label(voice: Option<&str>) -> String {
    match voice.and_then(find_gemini_voice) {
        Some(voice) => format!("{} - {}", voice.name, voice.style),
        None => "Automatica (Gemini)".to_string(),
    }
}

fn load_gemini_api_key_from_db(db: &pax_db::Database) -> Option<String> {
    db.get_app_preference(PREF_GEMINI_API_KEY)
        .ok()
        .flatten()
        .and_then(non_empty_trimmed)
}

fn save_gemini_api_key_to_db(db: &pax_db::Database, value: &str) -> Result<()> {
    match non_empty_trimmed(value.to_string()) {
        Some(value) => db.set_app_preference(PREF_GEMINI_API_KEY, &value),
        None => db.delete_app_preference(PREF_GEMINI_API_KEY),
    }
}

fn load_gemini_api_key_from_env() -> Option<String> {
    ["GEMINI_API_KEY", "GOOGLE_API_KEY"]
        .iter()
        .find_map(|key| std::env::var(key).ok().and_then(non_empty_trimmed))
}

fn load_gemini_model_from_db(db: &pax_db::Database) -> Option<String> {
    db.get_app_preference(PREF_GEMINI_MODEL)
        .ok()
        .flatten()
        .and_then(non_empty_trimmed)
}

fn save_gemini_model_to_db(db: &pax_db::Database, value: &str) -> Result<()> {
    match non_empty_trimmed(value.to_string()) {
        Some(value) => db.set_app_preference(PREF_GEMINI_MODEL, &value),
        None => db.delete_app_preference(PREF_GEMINI_MODEL),
    }
}

fn load_gemini_voice_from_db(db: &pax_db::Database) -> Option<String> {
    db.get_app_preference(PREF_GEMINI_VOICE)
        .ok()
        .flatten()
        .and_then(|voice| canonical_gemini_voice(&voice))
}

fn save_gemini_voice_to_db(db: &pax_db::Database, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return db.delete_app_preference(PREF_GEMINI_VOICE);
    }
    let Some(voice) = canonical_gemini_voice(value) else {
        anyhow::bail!("Voce Gemini non supportata: {value}");
    };
    db.set_app_preference(PREF_GEMINI_VOICE, &voice)
}

fn load_gemini_voice_from_env() -> Option<String> {
    std::env::var("PAX_VOICE_GEMINI_VOICE")
        .ok()
        .and_then(|voice| canonical_gemini_voice(&voice))
}

fn load_gemini_model_from_env() -> Option<String> {
    ["PAX_VOICE_GEMINI_MODEL", "GOOGLE_GENAI_MODEL_NAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok().and_then(non_empty_trimmed))
}

fn non_empty_trimmed(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn migrate_legacy_model(model: String) -> String {
    if model == LEGACY_BATCH_MODEL {
        DEFAULT_GEMINI_MODEL.to_string()
    } else {
        model
    }
}

fn canonical_gemini_voice(value: &str) -> Option<String> {
    find_gemini_voice(value)
        .map(|voice| voice.name)
        .map(str::to_string)
}

fn find_gemini_voice(value: &str) -> Option<&'static GeminiVoiceOption> {
    let value = value.trim();
    GEMINI_VOICE_OPTIONS
        .iter()
        .find(|voice| voice.name.eq_ignore_ascii_case(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_api_key_roundtrips_through_db() {
        let db = pax_db::Database::open_memory().unwrap();

        save_gemini_api_key_to_db(&db, "  secret-key  ").unwrap();

        assert_eq!(
            load_gemini_api_key_from_db(&db).as_deref(),
            Some("secret-key")
        );
    }

    #[test]
    fn empty_gemini_api_key_clears_db_value() {
        let db = pax_db::Database::open_memory().unwrap();

        save_gemini_api_key_to_db(&db, "secret-key").unwrap();
        save_gemini_api_key_to_db(&db, "   ").unwrap();

        assert_eq!(load_gemini_api_key_from_db(&db), None);
    }

    #[test]
    fn gemini_model_roundtrips_through_db() {
        let db = pax_db::Database::open_memory().unwrap();

        save_gemini_model_to_db(&db, " gemini-3-flash-preview ").unwrap();

        assert_eq!(
            load_gemini_model_from_db(&db).as_deref(),
            Some("gemini-3-flash-preview")
        );
    }

    #[test]
    fn legacy_batch_model_migrates_to_live_default() {
        assert_eq!(
            migrate_legacy_model(LEGACY_BATCH_MODEL.to_string()),
            DEFAULT_GEMINI_MODEL
        );
    }

    #[test]
    fn gemini_voice_roundtrips_canonical_name_through_db() {
        let db = pax_db::Database::open_memory().unwrap();

        save_gemini_voice_to_db(&db, "  puck  ").unwrap();

        assert_eq!(load_gemini_voice_from_db(&db).as_deref(), Some("Puck"));
    }

    #[test]
    fn automatic_gemini_voice_clears_the_preference() {
        let db = pax_db::Database::open_memory().unwrap();
        save_gemini_voice_to_db(&db, "Kore").unwrap();

        save_gemini_voice_to_db(&db, "").unwrap();

        assert_eq!(load_gemini_voice_from_db(&db), None);
    }

    #[test]
    fn gemini_voice_catalog_has_unique_names_and_stable_indexes() {
        let mut names = GEMINI_VOICE_OPTIONS
            .iter()
            .map(|voice| voice.name)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();

        assert_eq!(GEMINI_VOICE_OPTIONS.len(), 30);
        assert_eq!(names.len(), GEMINI_VOICE_OPTIONS.len());
        assert_eq!(gemini_voice_index(Some("Kore")), 4);
        assert_eq!(gemini_voice_at_index(4), Some("Kore"));
        assert_eq!(gemini_voice_at_index(0), None);
    }
}
