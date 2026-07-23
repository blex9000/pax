use serde::Serialize;
use serde_json::Value;

pub const REDACTED_VALUE: &str = "[REDACTED]";

pub fn redacted_json<T: Serialize>(value: &T) -> serde_json::Result<Value> {
    let mut value = serde_json::to_value(value)?;
    redact_json(&mut value);
    Ok(value)
}

pub fn redact_json(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                if is_sensitive_key(key) {
                    *value = Value::String(REDACTED_VALUE.to_string());
                } else {
                    redact_json(value);
                }
            }
        }
        Value::Array(values) => values.iter_mut().for_each(redact_json),
        _ => {}
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();

    matches!(
        normalized.as_str(),
        "password"
            | "passphrase"
            | "apikey"
            | "accesstoken"
            | "refreshtoken"
            | "authorization"
            | "cookie"
            | "secret"
            | "clientsecret"
            | "privatekey"
            | "env"
            | "environment"
    ) || normalized.ends_with("password")
        || normalized.ends_with("apikey")
        || normalized.ends_with("accesstoken")
        || normalized.ends_with("refreshtoken")
        || normalized.ends_with("secret")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recursively_redacts_credentials_and_environment() {
        let mut value = json!({
            "host": "example.test",
            "password": "pw",
            "gemini_api_key": "key",
            "nested": [{"access-token": "token", "name": "safe"}],
            "env": {"PATH": "/bin", "SAFE": "not-for-a-model"}
        });

        redact_json(&mut value);

        assert_eq!(value["host"], "example.test");
        assert_eq!(value["password"], REDACTED_VALUE);
        assert_eq!(value["gemini_api_key"], REDACTED_VALUE);
        assert_eq!(value["nested"][0]["access-token"], REDACTED_VALUE);
        assert_eq!(value["nested"][0]["name"], "safe");
        assert_eq!(value["env"], REDACTED_VALUE);
    }
}
