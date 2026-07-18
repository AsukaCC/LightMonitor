use anyhow::{Context, bail};
use std::fs;
use std::path::Path;
use uuid::Uuid;

pub const MAX_SSH_KEY_BYTES: usize = 1024 * 1024;

pub fn path_for(root: &Path, id: Uuid) -> std::path::PathBuf {
    root.join(id.to_string())
}

pub fn validate_name(name: &str) -> anyhow::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 128 || name.chars().any(|ch| ch.is_control()) {
        bail!("SSH key name must be 1-128 visible characters");
    }
    if name.contains('/') || name.contains('\\') {
        bail!("SSH key name cannot contain a path separator");
    }
    Ok(name.to_string())
}

pub fn validate_contents(contents: &[u8]) -> anyhow::Result<()> {
    if contents.is_empty() {
        bail!("SSH key file is empty");
    }
    if contents.len() > MAX_SSH_KEY_BYTES {
        bail!("SSH key file is too large");
    }
    std::str::from_utf8(contents).context("SSH key file must be UTF-8 text")?;
    Ok(())
}

pub fn write_private(path: &Path, contents: &[u8]) -> anyhow::Result<()> {
    validate_contents(contents)?;
    let temporary = path.with_extension(format!("upload-{}", Uuid::new_v4()));
    fs::write(&temporary, contents)
        .with_context(|| format!("write SSH key {}", temporary.display()))?;
    set_private_permissions(&temporary)?;
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("replace SSH key {}", path.display()))?;
    }
    fs::rename(&temporary, path).with_context(|| format!("store SSH key {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_key_names_and_contents() {
        assert_eq!(validate_name("production").unwrap(), "production");
        assert!(validate_name("../secret").is_err());
        assert!(validate_contents(b"-----BEGIN OPENSSH PRIVATE KEY-----").is_ok());
        assert!(validate_contents(b"").is_err());
        assert!(validate_contents(&vec![b'x'; MAX_SSH_KEY_BYTES + 1]).is_err());
    }
}
