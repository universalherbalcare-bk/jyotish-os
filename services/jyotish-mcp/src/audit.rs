//! Append-only JSONL audit log. One line per tool invocation:
//! `{ts, tool, request_hash, ms, status, cache_hit}`. Never contains request
//! payloads (birth data) or secrets — only the content hash.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

pub struct Audit {
    file: Mutex<File>,
}

impl Audit {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn record(&self, tool: &str, request_hash: &str, ms: u128, status: &str, cache_hit: bool) {
        let line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "tool": tool,
            "request_hash": request_hash,
            "ms": ms,
            "status": status,
            "cache_hit": cache_hit,
        });
        // A poisoned mutex means another writer panicked mid-write; we still
        // want subsequent lines, so recover the guard rather than abort.
        let mut guard = match self.file.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Err(e) = writeln!(guard, "{line}") {
            eprintln!("{{\"level\":\"error\",\"msg\":\"audit write failed\",\"error\":\"{e}\"}}");
        }
    }
}
