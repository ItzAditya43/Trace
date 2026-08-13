use serde::Serialize;
use std::collections::HashMap;
use std::fs;

#[derive(Serialize, Clone)]
pub struct NamespaceGroup {
    pub ns_type: String,
    pub inode: String,
    pub pids: Vec<u32>,
    pub process_names: Vec<String>,
}

const NS_TYPES: [&str; 7] = ["net", "pid", "mnt", "uts", "ipc", "user", "cgroup"];

/// Live equivalent of `lsns`: walks every process's /proc/[pid]/ns/* symlinks,
/// whose targets look like "net:[4026531840]", and groups PIDs that share the
/// same (type, inode) pair — meaning they're in the same namespace. Read-only,
/// no privilege needed for namespaces the calling user's own processes belong
/// to (cross-user namespaces may show fewer PIDs without root).
pub fn list_namespaces(sys: &sysinfo::System) -> Vec<NamespaceGroup> {
    let mut groups: HashMap<(String, String), Vec<u32>> = HashMap::new();

    let Ok(proc_entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    for entry in proc_entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(pid) = name.parse::<u32>() else { continue };
        let ns_dir = entry.path().join("ns");
        for ns_type in NS_TYPES {
            let Ok(link) = fs::read_link(ns_dir.join(ns_type)) else { continue };
            let link_str = link.to_string_lossy();
            if let Some(inode) = link_str
                .strip_prefix(&format!("{ns_type}:["))
                .and_then(|s| s.strip_suffix(']'))
            {
                groups
                    .entry((ns_type.to_string(), inode.to_string()))
                    .or_default()
                    .push(pid);
            }
        }
    }

    let mut result: Vec<NamespaceGroup> = groups
        .into_iter()
        .map(|((ns_type, inode), mut pids)| {
            pids.sort_unstable();
            pids.dedup();
            let process_names = pids
                .iter()
                .map(|&pid| {
                    sys.process(sysinfo::Pid::from_u32(pid))
                        .map(|p| p.name().to_string_lossy().to_string())
                        .unwrap_or_else(|| "?".to_string())
                })
                .collect();
            NamespaceGroup { ns_type, inode, pids, process_names }
        })
        .collect();

    // Namespaces shared by fewer processes are usually the interesting
    // (container/sandbox) ones — surface those first.
    result.sort_by(|a, b| {
        a.ns_type
            .cmp(&b.ns_type)
            .then(a.pids.len().cmp(&b.pids.len()))
    });
    result
}
