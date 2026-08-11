use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    name: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub release_url: String,
    pub release_name: String,
}

/// Compares dotted numeric versions like "0.1.42" > "0.1.7". Non-numeric
/// segments are ignored (treated as 0) rather than causing a hard failure,
/// since a malformed tag shouldn't crash the update check.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let l = parse(latest);
    let c = parse(current);
    for i in 0..l.len().max(c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv != cv {
            return lv > cv;
        }
    }
    false
}

pub async fn check_for_update(repo: &str) -> Result<Option<UpdateInfo>, String> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");

    let client = reqwest::Client::builder()
        .user_agent("trace-update-checker")
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Update check failed: {e}"))?;

    if !resp.status().is_success() {
        // No releases published yet, rate-limited, or offline — not an error
        // worth surfacing to the user, just means "nothing to report".
        return Ok(None);
    }

    let release: GithubRelease = resp.json().await.map_err(|e| e.to_string())?;
    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    if is_newer(&latest_version, &current_version) {
        Ok(Some(UpdateInfo {
            current_version,
            latest_version,
            release_url: release.html_url,
            release_name: release.name.unwrap_or(release.tag_name),
        }))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically_not_lexically() {
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(!is_newer("0.1.9", "0.1.10"));
        assert!(is_newer("0.2.0", "0.1.99"));
        assert!(!is_newer("0.1.5", "0.1.5"));
        assert!(is_newer("v1.0.0", "0.9.0"));
    }
}
