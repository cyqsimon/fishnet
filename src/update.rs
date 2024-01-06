use std::{fmt, io, io::Write as _};

use futures_util::StreamExt as _;
use reqwest::Client;
use self_replace::self_replace;
use semver::Version;
use serde::Deserialize;
use tempfile::NamedTempFile;

use crate::logger::Logger;

pub async fn auto_update(
    verbose: bool,
    client: &Client,
    logger: &Logger,
) -> Result<UpdateSuccess, UpdateError> {
    if verbose {
        logger.headline("Updating ...");
    }

    // Find relevant updates.
    logger.fishnet_info("Checking for updates (--auto-update) ...");
    let current = Version::parse(env!("CARGO_PKG_VERSION")).expect("valid package version");
    let latest = latest_release(client).await?;
    logger.debug(&format!(
        "Current release is v{}, latest is v{}",
        current, latest.version
    ));
    if latest.version <= current {
        return Ok(UpdateSuccess::UpToDate(current));
    }

    // Download.
    logger.fishnet_info("Downloading v{latest} ...");
    let mut temp_exe = NamedTempFile::with_prefix("fishnet-auto-update")?;
    let mut download = client
        .get(format!(
            "https://fishnet-releases.s3.dualstack.eu-west-3.amazonaws.com/{}",
            latest.key
        ))
        .send()
        .await?
        .error_for_status()?
        .bytes_stream();
    while let Some(part) = download.next().await {
        let part = part?;
        temp_exe.write_all(&part)?;
    }
    temp_exe.flush()?;

    // Replace current executable.
    self_replace(temp_exe)?;
    Ok(UpdateSuccess::Updated(latest.version))
}

async fn latest_release(client: &Client) -> Result<Release, UpdateError> {
    let bucket: ListBucket = quick_xml::de::from_str(
        &client
            .get("https://fishnet-releases.s3.dualstack.eu-west-3.amazonaws.com/?list-type=2")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?,
    )?;

    bucket
        .contents
        .into_iter()
        .flat_map(Content::release)
        .max()
        .ok_or(UpdateError::NoReleases)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ListBucket {
    contents: Vec<Content>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Content {
    key: String,
}

impl Content {
    fn release(self) -> Option<Release> {
        let (version, filename) = self.key.split_once('/')?;
        if !filename.contains(concat!("-", env!("FISHNET_TARGET"))) {
            return None;
        }
        let version = version.strip_prefix('v')?;
        Some(Release {
            version: version.parse().ok()?,
            key: self.key,
        })
    }
}

#[derive(Debug, Ord, PartialOrd, Eq, PartialEq)]
struct Release {
    version: Version,
    key: String,
}

pub enum UpdateSuccess {
    Updated(Version),
    UpToDate(Version),
}

#[derive(Debug)]
pub enum UpdateError {
    NoReleases,
    Network(reqwest::Error),
    Xml(quick_xml::DeError),
    Io(io::Error),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::NoReleases => f.write_str(concat!(
                "auto update not supported for ",
                env!("FISHNET_TARGET")
            )),
            UpdateError::Network(err) => write!(f, "{err}"),
            UpdateError::Xml(err) => write!(f, "unexpected response from aws: {err}"),
            UpdateError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl From<reqwest::Error> for UpdateError {
    fn from(err: reqwest::Error) -> UpdateError {
        UpdateError::Network(err)
    }
}

impl From<quick_xml::DeError> for UpdateError {
    fn from(err: quick_xml::DeError) -> UpdateError {
        UpdateError::Xml(err)
    }
}

impl From<io::Error> for UpdateError {
    fn from(err: io::Error) -> UpdateError {
        UpdateError::Io(err)
    }
}
