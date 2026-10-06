//! Update checks against GitHub releases for AEmad99/devterm.
//!
//! Port of `src/main/updater.ts` plus `electron-builder.yml` `publish`.
//! Development (`ELECTRON_RENDERER_URL` set) and `--self-test` skip the
//! update server. Packaged builds check GitHub releases.

/// GitHub owner / repo published by electron-builder.
pub const GITHUB_OWNER: &str = "AEmad99";
pub const GITHUB_REPO: &str = "devterm";

/// Feed electron-updater's GitHub provider uses for this app.
pub fn update_feed_url() -> &'static str {
    "https://github.com/AEmad99/devterm/releases/latest"
}

/// Latest-release API used when a caller wants the release list directly.
pub fn github_releases_api_url() -> &'static str {
    "https://api.github.com/repos/AEmad99/devterm/releases"
}

/// True when background and manual update checks are allowed.
/// `is_dev` is `ELECTRON_RENDERER_URL` being set. `is_self_test` is `--self-test`.
pub fn should_check_updates(is_dev: bool, is_self_test: bool) -> bool {
    !is_dev && !is_self_test
}

pub const DISABLED_MESSAGE: &str = "Update checks run in packaged installs only. Development and self-test builds skip the update server.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Disabled,
    Downloaded,
    Available,
    UpToDate,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateCheckResult {
    pub status: UpdateStatus,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub message: String,
}

/// Manual check result when the updater is disabled (dev or self-test).
pub fn disabled_check(current_version: &str) -> UpdateCheckResult {
    UpdateCheckResult {
        status: UpdateStatus::Disabled,
        current_version: current_version.to_string(),
        latest_version: None,
        message: DISABLED_MESSAGE.to_string(),
    }
}

/// Pure classification of a completed check, matching `checkForUpdatesNow`
/// once a latest version string is known. `download_started` mirrors
/// electron-updater setting `downloadPromise`.
pub fn classify_check(
    is_dev: bool,
    is_self_test: bool,
    current_version: &str,
    last_downloaded: Option<&str>,
    latest_version: Option<&str>,
    download_started: bool,
    error: Option<&str>,
) -> UpdateCheckResult {
    if !should_check_updates(is_dev, is_self_test) {
        return disabled_check(current_version);
    }
    if let Some(downloaded) = last_downloaded {
        if downloaded != current_version {
            return UpdateCheckResult {
                status: UpdateStatus::Downloaded,
                current_version: current_version.to_string(),
                latest_version: Some(downloaded.to_string()),
                message: format!(
                    "DevTerm {downloaded} is downloaded. Restart to install, or it will apply on next quit."
                ),
            };
        }
    }
    if let Some(message) = error {
        return UpdateCheckResult {
            status: UpdateStatus::Error,
            current_version: current_version.to_string(),
            latest_version: None,
            message: if message.is_empty() {
                "Update check failed.".to_string()
            } else {
                message.to_string()
            },
        };
    }
    let Some(latest) = latest_version.filter(|s| !s.is_empty()) else {
        return UpdateCheckResult {
            status: UpdateStatus::Error,
            current_version: current_version.to_string(),
            latest_version: None,
            message: "Update check returned no result. Check your network and try again."
                .to_string(),
        };
    };
    if download_started {
        return UpdateCheckResult {
            status: UpdateStatus::Available,
            current_version: current_version.to_string(),
            latest_version: Some(latest.to_string()),
            message: format!(
                "Version {latest} is available and downloading in the background. You will be prompted when it is ready."
            ),
        };
    }
    if latest != current_version {
        return UpdateCheckResult {
            status: UpdateStatus::Available,
            current_version: current_version.to_string(),
            latest_version: Some(latest.to_string()),
            message: format!("Version {latest} is available."),
        };
    }
    UpdateCheckResult {
        status: UpdateStatus::UpToDate,
        current_version: current_version.to_string(),
        latest_version: Some(latest.to_string()),
        message: "You are running the latest version.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_in_dev_and_self_test() {
        assert!(!should_check_updates(true, false));
        assert!(!should_check_updates(false, true));
        assert!(!should_check_updates(true, true));
        assert!(should_check_updates(false, false));
    }

    #[test]
    fn feed_points_at_aemad99_devterm_releases() {
        let url = update_feed_url();
        assert!(url.contains("AEmad99/devterm"));
        assert!(url.contains("github.com"));
        assert!(github_releases_api_url().ends_with("/repos/AEmad99/devterm/releases"));
        assert_eq!(GITHUB_OWNER, "AEmad99");
        assert_eq!(GITHUB_REPO, "devterm");
    }

    #[test]
    fn disabled_message_matches_updater_ts() {
        let r = classify_check(true, false, "1.6.7", None, None, false, None);
        assert_eq!(r.status, UpdateStatus::Disabled);
        assert_eq!(r.message, DISABLED_MESSAGE);
        assert_eq!(r.current_version, "1.6.7");
    }

    #[test]
    fn downloaded_and_up_to_date_copy() {
        let d = classify_check(false, false, "1.6.6", Some("1.6.7"), None, false, None);
        assert_eq!(d.status, UpdateStatus::Downloaded);
        assert!(d.message.contains("DevTerm 1.6.7 is downloaded"));
        let u = classify_check(false, false, "1.6.7", None, Some("1.6.7"), false, None);
        assert_eq!(u.status, UpdateStatus::UpToDate);
        assert_eq!(u.message, "You are running the latest version.");
    }
}
