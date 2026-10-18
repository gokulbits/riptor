//! API keys live in the OS credential store (Windows Credential Manager,
//! Secret Service on Linux), never in Riptor's config files or backups.

const SERVICE: &str = "Riptor";

fn entry(id: &str) -> anyhow::Result<keyring::Entry> {
    Ok(keyring::Entry::new(SERVICE, &format!("torznab:{id}"))?)
}

pub fn store_api_key(id: &str, key: &str) -> anyhow::Result<()> {
    entry(id)?.set_password(key).map_err(|e| {
        anyhow::anyhow!("could not save the API key in the system credential store: {e}")
    })
}

pub fn api_key(id: &str) -> anyhow::Result<String> {
    entry(id)?
        .get_password()
        .map_err(|e| anyhow::anyhow!("API key not found in the system credential store: {e}"))
}

pub fn delete_api_key(id: &str) {
    if let Ok(e) = entry(id) {
        let _ = e.delete_credential();
    }
}
