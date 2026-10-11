use herdr_relay::config::{Config, PUBLIC_ORIGIN};
use std::os::unix::fs::PermissionsExt;

const ORIGIN: &str = "https://relay.example.org";
const TOKEN: &str = "correct-horse-battery-staple-42";
/// Serializes tests that change the process-wide configuration or environment.
static GLOBAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn a_missing_file_is_an_empty_configuration() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        Config::load(&dir.path().join("config.toml")).unwrap(),
        Config::default()
    );
}

#[test]
fn settings_round_trip_through_a_private_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/config.toml");
    let mut config = Config::default();
    config.set("origin", ORIGIN, None).unwrap();
    config.set("herdr", "/opt/herdr/bin/herdr", None).unwrap();
    config.set("profile", "/var/relay-profile", None).unwrap();
    config.set("update-repository", "me/fork", None).unwrap();
    config
        .set("update-url", "https://mirror.example.org", None)
        .unwrap();
    config.set("direct", "off", None).unwrap();
    config.set("direct-port", "47999", None).unwrap();
    config.set("token", TOKEN, Some(ORIGIN)).unwrap();
    config.save(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("origin = \"https://relay.example.org\""),
        "{text}"
    );
    assert!(text.contains("[tokens]"), "{text}");
    // File keys match the names `config set` uses.
    assert!(text.contains("direct-port = 47999"), "{text}");
    assert!(text.contains("update-repository = \"me/fork\""), "{text}");
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded, config);
    assert_eq!(loaded.origin.as_deref(), Some(ORIGIN));
    assert_eq!(loaded.direct, Some(false));
    assert_eq!(
        loaded.update_url.as_deref(),
        Some("https://mirror.example.org")
    );
    assert_eq!(loaded.direct_port, Some(47999));
    assert_eq!(loaded.token(ORIGIN), Some(TOKEN));
    assert_eq!(loaded.token("https://other.example"), None);
    // Showing the configuration never prints a token.
    let shown = loaded.show();
    assert!(!shown.contains(TOKEN), "{shown}");
    assert!(shown.contains(ORIGIN), "{shown}");
}

#[test]
fn unset_removes_a_setting() {
    let mut config = Config::default();
    config.set("origin", ORIGIN, None).unwrap();
    config.set("token", TOKEN, Some(ORIGIN)).unwrap();
    config.unset("origin", None).unwrap();
    config.unset("token", Some(ORIGIN)).unwrap();
    assert_eq!(config, Config::default());
}

#[test]
fn invalid_settings_are_refused() {
    let mut config = Config::default();
    for (key, value) in [
        ("origin", "http://relay.example.org"),
        ("direct", "sometimes"),
        ("direct-port", "0"),
        ("direct-port", "70000"),
        ("update-repository", "not a repository"),
        ("update-url", "http://mirror.example.org"),
        ("update-url", "https://mirror.example.org/path"),
        ("colour", "blue"),
    ] {
        assert!(config.set(key, value, None).is_err(), "{key}={value}");
    }
    for token in ["short", "has a space in the middle of it"] {
        assert!(config.set("token", token, Some(ORIGIN)).is_err(), "{token}");
    }
    let error = config.set("token", TOKEN, Some(PUBLIC_ORIGIN)).unwrap_err();
    assert_eq!(error.code, "PUBLIC_RELAY");
    assert!(config.set("token", TOKEN, None).is_err());
    assert_eq!(config, Config::default());
}

#[test]
fn an_unknown_or_malformed_file_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    for text in [
        "colour = \"blue\"\n",
        "origin = [\n",
        "origin = \"http://insecure\"\n",
    ] {
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            Config::load(&path).unwrap_err().code,
            "CONFIG_INVALID",
            "{text}"
        );
    }
    // A token configured for the public relay is never accepted from a file either.
    std::fs::write(
        &path,
        format!("[tokens]\n\"{PUBLIC_ORIGIN}\" = \"{TOKEN}\"\n"),
    )
    .unwrap();
    assert_eq!(Config::load(&path).unwrap_err().code, "CONFIG_INVALID");
}

#[test]
fn a_hosts_direct_listener_follows_the_configuration_below_the_environment() {
    let _global = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    use herdr_relay::{config::use_config, direct::configured_port};
    unsafe {
        std::env::remove_var("HERDR_RELAY_DIRECT");
        std::env::remove_var("HERDR_RELAY_DIRECT_PORT");
    }
    let mut config = Config::default();
    config.set("direct-port", "47999", None).unwrap();
    use_config(config.clone());
    assert_eq!(configured_port(), Some(47999));
    config.set("direct", "off", None).unwrap();
    use_config(config);
    assert_eq!(configured_port(), None);
    unsafe { std::env::set_var("HERDR_RELAY_DIRECT", "on") };
    assert_eq!(configured_port(), Some(47999));
    unsafe { std::env::set_var("HERDR_RELAY_DIRECT_PORT", "48000") };
    assert_eq!(configured_port(), Some(48000));
    unsafe {
        std::env::remove_var("HERDR_RELAY_DIRECT");
        std::env::remove_var("HERDR_RELAY_DIRECT_PORT");
    }
    use_config(Config::default());
}

#[test]
fn a_token_entered_interactively_is_saved_and_used_at_once() {
    let _global = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    use herdr_relay::config::{access_token, current, save_token};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "origin = \"https://relay.example.org\"\n").unwrap();
    save_token(&path, ORIGIN, TOKEN).unwrap();
    let saved = Config::load(&path).unwrap();
    assert_eq!(
        saved.origin.as_deref(),
        Some(ORIGIN),
        "other settings are kept"
    );
    assert_eq!(saved.token(ORIGIN), Some(TOKEN));
    assert_eq!(current().token(ORIGIN), Some(TOKEN));
    if std::env::var_os("HERDR_RELAY_TOKEN").is_none() {
        assert_eq!(access_token(ORIGIN).as_deref(), Some(TOKEN));
    }
    assert_eq!(
        save_token(&path, PUBLIC_ORIGIN, TOKEN).unwrap_err().code,
        "PUBLIC_RELAY"
    );
}
