//! Locating a Chromium-based browser and printing HTML to PDF with it.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Find a Chrome/Chromium binary. `explicit` (from `--chrome`) wins, then the
/// `PRINTCRAFT_CHROME` / `CHROME` environment variables, then well-known
/// install locations, then `PATH`.
pub fn find(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if path.is_file() {
            return Ok(path.to_path_buf());
        }
        bail!("--chrome {} does not exist", path.display());
    }
    for var in ["PRINTCRAFT_CHROME", "CHROME"] {
        if let Some(path) = env::var_os(var).map(PathBuf::from)
            && path.is_file()
        {
            return Ok(path);
        }
    }

    let mut candidates: Vec<PathBuf> = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Some(home) = env::var_os("HOME") {
        candidates.push(
            Path::new(&home).join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        );
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"] {
        if let Some(base) = env::var_os(var) {
            let base = PathBuf::from(base);
            candidates.push(base.join(r"Google\Chrome\Application\chrome.exe"));
            candidates.push(base.join(r"Microsoft\Edge\Application\msedge.exe"));
        }
    }
    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Ok(found);
    }

    let names = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "chrome.exe",
        "msedge",
    ];
    let path = env::var_os("PATH").unwrap_or_else(OsString::new);
    for dir in env::split_paths(&path) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!(
        "could not find Google Chrome or Chromium; install one or pass --chrome <path> \
         (or set PRINTCRAFT_CHROME)"
    )
}

/// Print `html` to `pdf` with headless Chrome. Chrome emits a tagged PDF and,
/// with `--generate-pdf-document-outline`, bookmarks built from the headings.
pub fn print_to_pdf(chrome: &Path, html: &Path, pdf: &Path, profile_dir: &Path) -> Result<()> {
    if pdf.exists() {
        fs::remove_file(pdf).with_context(|| format!("removing stale {}", pdf.display()))?;
    }
    let mut print_arg = OsString::from("--print-to-pdf=");
    print_arg.push(pdf);
    let mut profile_arg = OsString::from("--user-data-dir=");
    profile_arg.push(profile_dir);

    let output = Command::new(chrome)
        .arg("--headless=new")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--allow-file-access-from-files")
        .arg("--no-pdf-header-footer")
        .arg("--generate-pdf-document-outline")
        .arg(profile_arg)
        .arg(print_arg)
        .arg(super::file_url(html))
        .output()
        .with_context(|| format!("running {}", chrome.display()))?;

    let ok = output.status.success() && fs::metadata(pdf).map(|m| m.len() > 0).unwrap_or(false);
    if !ok {
        bail!(
            "Chrome did not produce {} (status {}):\n{}",
            pdf.display(),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}
