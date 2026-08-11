use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Serialize, Clone)]
pub struct ProjectInfo {
    pub name: String,
    pub path: String,
    pub languages: Vec<String>,
    pub has_git: bool,
    pub commit_count: u32,
    pub last_commit_days_ago: Option<i64>,
    pub todo_count: u32,
    pub disk_bytes: u64,
    pub dependency_count: Option<u32>,
    pub activity: String,
}

fn run(cmd: &mut Command) -> Option<String> {
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn detect_languages(dir: &Path) -> Vec<String> {
    let mut langs = Vec::new();
    let markers: [(&str, &str); 7] = [
        ("Cargo.toml", "Rust"),
        ("package.json", "JavaScript/TypeScript"),
        ("requirements.txt", "Python"),
        ("pyproject.toml", "Python"),
        ("go.mod", "Go"),
        ("pom.xml", "Java"),
        ("Gemfile", "Ruby"),
    ];
    for (file, lang) in markers {
        if dir.join(file).exists() && !langs.contains(&lang.to_string()) {
            langs.push(lang.to_string());
        }
    }
    langs
}

fn count_todos(dir: &Path) -> u32 {
    let out = Command::new("rg")
        .args(["-i", "-c", "TODO|FIXME", "--no-messages"])
        .arg(dir)
        .output();
    let Ok(out) = out else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.rsplit(':').next()?.parse::<u32>().ok())
        .sum()
}

fn count_dependencies(dir: &Path) -> Option<u32> {
    if let Ok(text) = fs::read_to_string(dir.join("package.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            let mut count = 0u32;
            for key in ["dependencies", "devDependencies"] {
                if let Some(obj) = json.get(key).and_then(|v| v.as_object()) {
                    count += obj.len() as u32;
                }
            }
            return Some(count);
        }
    }
    if let Ok(text) = fs::read_to_string(dir.join("requirements.txt")) {
        let count = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
            .count() as u32;
        return Some(count);
    }
    if let Ok(text) = fs::read_to_string(dir.join("Cargo.toml")) {
        let mut in_deps = false;
        let mut count = 0u32;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') {
                in_deps = trimmed.starts_with("[dependencies")
                    && !trimmed.starts_with("[dependencies.");
                continue;
            }
            if in_deps && trimmed.contains('=') && !trimmed.is_empty() {
                count += 1;
            }
        }
        return Some(count);
    }
    None
}

fn dir_size_bytes(dir: &Path) -> u64 {
    run(Command::new("du").args(["-sb"]).arg(dir))
        .and_then(|s| s.split_whitespace().next().map(|s| s.to_string()))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

pub fn scan(root: &str) -> Vec<ProjectInfo> {
    let root_path = Path::new(root);
    let Ok(entries) = fs::read_dir(root_path) else {
        return Vec::new();
    };

    let mut projects = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let has_git = path.join(".git").exists();
        let mut commit_count = 0u32;
        let mut last_commit_days_ago = None;

        if has_git {
            commit_count = run(Command::new("git")
                .args(["-C"])
                .arg(&path)
                .args(["rev-list", "--count", "HEAD"]))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            last_commit_days_ago = run(Command::new("git")
                .args(["-C"])
                .arg(&path)
                .args(["log", "-1", "--format=%ct"]))
                .and_then(|s| s.parse::<i64>().ok())
                .map(|ts| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64;
                    (now - ts) / 86400
                });
        }

        let activity = match (has_git, last_commit_days_ago) {
            (true, Some(d)) if d <= 7 => "active",
            (true, Some(d)) if d <= 60 => "dormant",
            (true, Some(_)) => "abandoned",
            (true, None) => "dormant",
            (false, _) => "no_git",
        }
        .to_string();

        projects.push(ProjectInfo {
            name,
            path: path.to_string_lossy().to_string(),
            languages: detect_languages(&path),
            has_git,
            commit_count,
            last_commit_days_ago,
            todo_count: count_todos(&path),
            disk_bytes: dir_size_bytes(&path),
            dependency_count: count_dependencies(&path),
            activity,
        });
    }

    projects
}
