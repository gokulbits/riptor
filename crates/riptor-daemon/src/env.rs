//! Settings taken from `RIPTOR_*` environment variables, so a container can be set up without
//! editing `settings.json`. A variable that is set wins over the file at every start.

use std::path::PathBuf;

use anyhow::{bail, ensure, Context};
use riptor_core::{Settings, WatchedFolder};

/// Longest password file read; real ones are a line.
const MAX_PASSWORD_FILE: u64 = 4096;

fn path(var: &str, v: &str) -> anyhow::Result<PathBuf> {
    let p = PathBuf::from(v);
    ensure!(p.is_absolute(), "{var} must be an absolute path");
    Ok(p)
}

fn flag(var: &str, v: &str) -> anyhow::Result<bool> {
    Ok(match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => bail!("{var} must be on or off"),
    })
}

fn port(var: &str, v: &str) -> anyhow::Result<u16> {
    match v.trim().parse::<u16>() {
        Ok(p) if p > 0 => Ok(p),
        _ => bail!("{var} must be a port from 1 to 65535"),
    }
}

/// Applies every variable that's set; the engine validates the result like any other change.
pub fn apply(s: &mut Settings, get: impl Fn(&str) -> Option<String>) -> anyhow::Result<()> {
    let get = |var: &str| get(var).filter(|v| !v.trim().is_empty());
    if let Some(v) = get("RIPTOR_DOWNLOAD_DIR") {
        s.download_dir = path("RIPTOR_DOWNLOAD_DIR", &v)?;
    }
    if let Some(v) = get("RIPTOR_INCOMPLETE_DIR") {
        s.incomplete_dir = Some(path("RIPTOR_INCOMPLETE_DIR", &v)?);
    }
    if let Some(v) = get("RIPTOR_WATCH_DIR") {
        let p = path("RIPTOR_WATCH_DIR", &v)?;
        if !s.watched_folders.iter().any(|w| w.path == p) {
            s.watched_folders.push(WatchedFolder {
                path: p,
                category: None,
                save_path: None,
            });
        }
    }
    if let Some(v) = get("RIPTOR_PEER_PORT") {
        s.listen_port = port("RIPTOR_PEER_PORT", &v)?;
    }
    let w = &mut s.web_ui;
    if let Some(v) = get("RIPTOR_WEBUI") {
        w.enabled = flag("RIPTOR_WEBUI", &v)?;
    }
    if let Some(v) = get("RIPTOR_WEBUI_ADDRESS") {
        w.address = v.trim().to_string();
    }
    if let Some(v) = get("RIPTOR_WEBUI_PORT") {
        w.port = port("RIPTOR_WEBUI_PORT", &v)?;
    }
    if let Some(v) = get("RIPTOR_WEBUI_USERNAME") {
        w.username = v.trim().to_string();
    }
    if let Some(v) = get("RIPTOR_WEBUI_HTTPS") {
        w.https = flag("RIPTOR_WEBUI_HTTPS", &v)?;
    }
    if let Some(v) = get("RIPTOR_WEBUI_PLUGIN_INSTALL") {
        w.allow_plugin_install = flag("RIPTOR_WEBUI_PLUGIN_INSTALL", &v)?;
    }
    if let Some(v) = get("RIPTOR_WEBUI_TRUSTED_PROXIES") {
        w.reverse_proxy = true;
        w.trusted_proxies = v.trim().to_string();
    }
    Ok(())
}

/// `RIPTOR_WEBUI_PASSWORD`, or the first line of the file `RIPTOR_WEBUI_PASSWORD_FILE` names.
pub fn password(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Option<String>> {
    let direct = get("RIPTOR_WEBUI_PASSWORD").filter(|v| !v.is_empty());
    let file = get("RIPTOR_WEBUI_PASSWORD_FILE").filter(|v| !v.trim().is_empty());
    match (direct, file) {
        (Some(_), Some(_)) => {
            bail!("set RIPTOR_WEBUI_PASSWORD or RIPTOR_WEBUI_PASSWORD_FILE, not both")
        }
        (Some(p), None) => Ok(Some(p)),
        (None, Some(f)) => {
            use std::io::Read;
            let f = path("RIPTOR_WEBUI_PASSWORD_FILE", &f)?;
            let mut text = String::new();
            std::fs::File::open(&f)
                .and_then(|h| h.take(MAX_PASSWORD_FILE).read_to_string(&mut text))
                .with_context(|| format!("can't read {}", f.display()))?;
            let line = text.lines().next().unwrap_or("").to_string();
            ensure!(!line.is_empty(), "{} is empty", f.display());
            Ok(Some(line))
        }
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn vars(list: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = list
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn root(p: &str) -> String {
        std::env::temp_dir().join(p).display().to_string()
    }

    #[test]
    fn nothing_set_changes_nothing() {
        let mut s = Settings::default();
        apply(&mut s, vars(&[("RIPTOR_PEER_PORT", "  ")])).unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(password(vars(&[])).unwrap(), None);
    }

    #[test]
    fn container_variables_apply() {
        let (dl, inc, watch) = (root("dl"), root("inc"), root("watch"));
        let mut s = Settings::default();
        let env = vars(&[
            ("RIPTOR_DOWNLOAD_DIR", &dl),
            ("RIPTOR_INCOMPLETE_DIR", &inc),
            ("RIPTOR_WATCH_DIR", &watch),
            ("RIPTOR_PEER_PORT", "6881"),
            ("RIPTOR_WEBUI", "on"),
            ("RIPTOR_WEBUI_PORT", "8443"),
            ("RIPTOR_WEBUI_USERNAME", "me"),
            ("RIPTOR_WEBUI_HTTPS", "false"),
            ("RIPTOR_WEBUI_TRUSTED_PROXIES", "172.16.0.0/12"),
            ("RIPTOR_WEBUI_PLUGIN_INSTALL", "yes"),
        ]);
        apply(&mut s, &env).unwrap();
        // Twice, as at a restart: the watched folder isn't added again.
        apply(&mut s, &env).unwrap();
        assert_eq!(s.download_dir, PathBuf::from(&dl));
        assert_eq!(s.incomplete_dir, Some(PathBuf::from(&inc)));
        assert_eq!(s.watched_folders.len(), 1);
        assert_eq!(s.listen_port, 6881);
        assert!(s.web_ui.enabled && !s.web_ui.https && s.web_ui.reverse_proxy);
        assert!(s.web_ui.allow_plugin_install);
        assert_eq!((s.web_ui.port, s.web_ui.username.as_str()), (8443, "me"));
        s.validate().unwrap();
    }

    #[test]
    fn bad_values_are_refused() {
        for (k, v) in [
            ("RIPTOR_DOWNLOAD_DIR", "downloads"),
            ("RIPTOR_WATCH_DIR", "../watch"),
            ("RIPTOR_PEER_PORT", "0"),
            ("RIPTOR_PEER_PORT", "70000"),
            ("RIPTOR_WEBUI_PORT", "http"),
            ("RIPTOR_WEBUI", "maybe"),
            ("RIPTOR_WEBUI_PLUGIN_INSTALL", "sometimes"),
        ] {
            let mut s = Settings::default();
            assert!(apply(&mut s, vars(&[(k, v)])).is_err(), "{k}={v}");
        }
        // Left for the engine's validation, which refuses it.
        let mut s = Settings::default();
        apply(&mut s, vars(&[("RIPTOR_WEBUI_ADDRESS", "evil.example")])).unwrap();
        assert!(s.validate().is_err());
    }

    #[test]
    fn passwords_come_from_one_place() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("pw");
        std::fs::write(&f, "from-a-secret\nignored\n").unwrap();
        let file = f.display().to_string();
        assert_eq!(
            password(vars(&[("RIPTOR_WEBUI_PASSWORD_FILE", &file)])).unwrap(),
            Some("from-a-secret".into())
        );
        assert_eq!(
            password(vars(&[("RIPTOR_WEBUI_PASSWORD", "direct-one")])).unwrap(),
            Some("direct-one".into())
        );
        let both = vars(&[
            ("RIPTOR_WEBUI_PASSWORD", "x"),
            ("RIPTOR_WEBUI_PASSWORD_FILE", &file),
        ]);
        assert!(password(both).is_err());
        assert!(password(vars(&[("RIPTOR_WEBUI_PASSWORD_FILE", "pw")])).is_err());
        std::fs::write(&f, "\n").unwrap();
        assert!(password(vars(&[("RIPTOR_WEBUI_PASSWORD_FILE", &file)])).is_err());
    }
}
