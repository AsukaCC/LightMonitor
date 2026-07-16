use crate::models::{AppRelease, ReleaseCatalog};
use crate::state::AppState;
use anyhow::{Context, anyhow, bail};
use flate2::read::GzDecoder;
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use tar::Archive;
use uuid::Uuid;

const MAX_BUNDLE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    name: Option<String>,
    published_at: Option<String>,
    html_url: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

pub async fn release_catalog(state: &AppState) -> anyhow::Result<ReleaseCatalog> {
    let releases = fetch_releases(&state.config.github_repo).await?;
    let current = env!("CARGO_PKG_VERSION").to_string();
    let expected_asset = platform_asset_name();
    let mut catalog_releases = Vec::new();

    for release in releases.into_iter().filter(|release| !release.draft) {
        let asset = expected_asset
            .as_ref()
            .and_then(|expected| release.assets.iter().find(|asset| asset.name == *expected));
        let version = normalize_version(&release.tag_name);
        catalog_releases.push(AppRelease {
            installed: valid_version_dir(&version_dir(state, &version)),
            active: version == current,
            version,
            name: release.name.unwrap_or(release.tag_name),
            published_at: release.published_at,
            html_url: release.html_url,
            prerelease: release.prerelease,
            asset_name: asset.map(|asset| asset.name.clone()),
            asset_size: asset.map(|asset| asset.size),
        });
    }

    let latest_version = catalog_releases
        .iter()
        .find(|release| !release.prerelease && release.asset_name.is_some())
        .map(|release| release.version.clone());

    Ok(ReleaseCatalog {
        current_version: current,
        latest_version,
        github_repo: state.config.github_repo.clone(),
        managed_updates: state.config.managed_updates,
        platform_asset: expected_asset,
        releases: catalog_releases,
    })
}

pub async fn install_and_activate(state: &AppState, requested: &str) -> anyhow::Result<String> {
    let requested = normalize_version(requested);
    validate_version(&requested)?;
    let asset_name = platform_asset_name()
        .ok_or_else(|| anyhow!("managed updates are not available for this platform"))?;
    let releases = fetch_releases(&state.config.github_repo).await?;
    let release = releases
        .iter()
        .find(|release| !release.draft && normalize_version(&release.tag_name) == requested)
        .ok_or_else(|| anyhow!("version {requested} is no longer available on GitHub Releases"))?;
    let bundle = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| anyhow!("release {} does not contain {asset_name}", release.tag_name))?;

    if bundle.size > MAX_BUNDLE_BYTES {
        bail!("release bundle is larger than the 256 MiB safety limit");
    }

    let destination = version_dir(state, &requested);
    if !valid_version_dir(&destination) {
        download_and_unpack(release, bundle, &destination).await?;
    }

    let current = env!("CARGO_PKG_VERSION");
    if current != requested {
        atomic_write(&state.config.data_dir.join("previous-version"), current)?;
    }
    atomic_write(&state.config.data_dir.join("active-version"), &requested)?;
    Ok(requested)
}

async fn fetch_releases(repo: &str) -> anyhow::Result<Vec<GithubRelease>> {
    if repo.split('/').count() != 2 || repo.chars().any(char::is_whitespace) {
        bail!("LIGHTMONITOR_GITHUB_REPO must use owner/repository format");
    }
    let response = github_client()?
        .get(format!(
            "https://api.github.com/repos/{repo}/releases?per_page=100"
        ))
        .send()
        .await
        .context("failed to query GitHub Releases")?
        .error_for_status()
        .context("GitHub Releases returned an error")?;
    response
        .json::<Vec<GithubRelease>>()
        .await
        .context("invalid GitHub Releases response")
}

async fn download_and_unpack(
    release: &GithubRelease,
    bundle: &GithubAsset,
    destination: &Path,
) -> anyhow::Result<()> {
    let checksum_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "SHA256SUMS.txt")
        .ok_or_else(|| anyhow!("release {} has no SHA256SUMS.txt", release.tag_name))?;
    let client = github_client()?;
    let checksum_text = download_text(&client, checksum_asset).await?;
    let expected_hash = checksum_for(&checksum_text, &bundle.name)
        .ok_or_else(|| anyhow!("SHA256SUMS.txt has no entry for {}", bundle.name))?;
    let bytes = download_bytes(&client, bundle).await?;
    let actual_hash = format!("{:x}", Sha256::digest(&bytes));
    if !actual_hash.eq_ignore_ascii_case(&expected_hash) {
        bail!("checksum verification failed for {}", bundle.name);
    }

    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("invalid versions directory"))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".staging-{}", Uuid::new_v4()));
    let bytes_for_unpack = bytes;
    let staging_for_unpack = staging.clone();
    tokio::task::spawn_blocking(move || unpack_bundle(&bytes_for_unpack, &staging_for_unpack))
        .await
        .context("release unpack task failed")??;

    if !valid_version_dir(&staging) {
        let _ = fs::remove_dir_all(&staging);
        bail!("release bundle is missing lightmonitor-server or web/index.html");
    }

    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    fs::rename(&staging, destination)?;
    Ok(())
}

fn unpack_bundle(bytes: &[u8], destination: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(destination)?;
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = Archive::new(decoder);
    for entry in archive.entries().context("invalid release archive")? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            bail!("release archive contains an unsafe path");
        }
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            bail!("release archive contains an unsupported entry type");
        }
        entry.unpack_in(destination)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            destination.join("lightmonitor-server"),
            fs::Permissions::from_mode(0o755),
        )?;
    }
    Ok(())
}

async fn download_text(client: &Client, asset: &GithubAsset) -> anyhow::Result<String> {
    client
        .get(&asset.browser_download_url)
        .send()
        .await
        .with_context(|| format!("failed to download {}", asset.name))?
        .error_for_status()?
        .text()
        .await
        .with_context(|| format!("failed to read {}", asset.name))
}

async fn download_bytes(client: &Client, asset: &GithubAsset) -> anyhow::Result<Vec<u8>> {
    Ok(client
        .get(&asset.browser_download_url)
        .send()
        .await
        .with_context(|| format!("failed to download {}", asset.name))?
        .error_for_status()?
        .bytes()
        .await
        .with_context(|| format!("failed to read {}", asset.name))?
        .to_vec())
}

fn github_client() -> anyhow::Result<Client> {
    Client::builder()
        .user_agent(format!("LightMonitor/{}", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .context("failed to create GitHub client")
}

fn platform_asset_name() -> Option<String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("lightmonitor-app-linux-x86_64.tar.gz".to_string()),
        ("linux", "aarch64") => Some("lightmonitor-app-linux-aarch64.tar.gz".to_string()),
        _ => None,
    }
}

fn normalize_version(version: &str) -> String {
    version.trim().trim_start_matches('v').to_string()
}

fn validate_version(version: &str) -> anyhow::Result<()> {
    if version.is_empty()
        || version.len() > 64
        || !version.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
    {
        bail!("invalid release version");
    }
    Ok(())
}

fn version_dir(state: &AppState, version: &str) -> PathBuf {
    state.config.versions_dir.join(version)
}

fn valid_version_dir(path: &Path) -> bool {
    path.join("lightmonitor-server").is_file() && path.join("web/index.html").is_file()
}

fn checksum_for(contents: &str, filename: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let name = fields.next()?.trim_start_matches('*');
        (name == filename && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| hash.to_string())
    })
}

fn atomic_write(path: &Path, value: &str) -> anyhow::Result<()> {
    let temp = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    fs::write(&temp, format!("{}\n", value.trim()))?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_release_versions() {
        assert!(validate_version("1.2.3").is_ok());
        assert!(validate_version("1.2.3-rc.1").is_ok());
        assert!(validate_version("../bad").is_err());
        assert!(validate_version("bad/tag").is_err());
    }

    #[test]
    fn reads_named_checksum_only() {
        let checksums = concat!(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  one.tar.gz\n",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb *two.tar.gz\n",
        );
        assert_eq!(checksum_for(checksums, "two.tar.gz"), Some("b".repeat(64)));
        assert_eq!(checksum_for(checksums, "missing.tar.gz"), None);
    }
}
