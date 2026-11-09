//! Downloading one file to disk, with progress, cancelling and an optional sha256 check. Used for
//! game files, uv (the Python installer) and updates to Toki Hub itself.

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::sources::{github_error, Download};
use crate::{fail, HubError, Result};

const CHUNK: usize = 256 * 1024;

/// Download `plan` to `dest`. On any failure (including a cancel), `dest` is deleted.
pub fn fetch(
    agent: &ureq::Agent,
    plan: &Download,
    dest: &Path,
    progress: &dyn Fn(u64, u64),
    check_cancel: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let result = receive(agent, plan, dest, progress, check_cancel);
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

fn receive(
    agent: &ureq::Agent,
    plan: &Download,
    dest: &Path,
    progress: &dyn Fn(u64, u64),
    check_cancel: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let mut request = agent.get(&plan.url);
    for (key, value) in &plan.headers {
        request = request.header(*key, value);
    }
    let response = request
        .call()
        .map_err(|e| HubError(format!("Could not connect to download {}: {e}", plan.filename)))?;
    if !response.status().is_success() {
        if !plan.github_what.is_empty() {
            return Err(github_error(&response, &plan.github_what));
        }
        let status = response.status();
        return fail(format!(
            "Download failed: HTTP {} {} ({})",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            plan.url
        ));
    }
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    progress(0, total);
    let mut reader = response.into_body().into_reader();
    if let Some(folder) = dest.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut out = File::create(dest)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; CHUNK];
    let mut received = 0u64;
    loop {
        check_cancel()?;
        let count = reader
            .read(&mut buffer)
            .map_err(|e| HubError(format!("The download was interrupted: {e}")))?;
        if count == 0 {
            break;
        }
        out.write_all(&buffer[..count])?;
        digest.update(&buffer[..count]);
        received += count as u64;
        progress(received, total);
    }
    out.flush()?;
    if total > 0 && received < total {
        return fail("The download ended early. Please try again.");
    }
    if !plan.sha256.is_empty() && hex(&digest.finalize()) != plan.sha256.to_lowercase() {
        return fail(format!(
            "{} does not match its sha256, so it was not used. The file may be damaged, or the catalog entry needs updating.",
            plan.filename
        ));
    }
    Ok(())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
