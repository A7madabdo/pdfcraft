//! `cargo xtask demo-pdf`: build the PrintCraft showcase PDF.
//!
//! 1. Render a procedural PNG and fill the placeholders in `assets/demo/showcase.html`.
//! 2. Print the HTML with headless Chrome (tagged PDF, outline from headings).
//! 3. Post-process with `lopdf`: turn marker links into annotations, append an
//!    AcroForm page, add layers, page labels, attachments, metadata and outline.

mod annots;
mod chrome;
mod form;
mod pdf;
mod postprocess;
mod raster;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

const USAGE: &str = "Usage: cargo xtask demo-pdf [--chrome <path>] [--out <file.pdf>]";

pub fn run(args: &[String]) -> Result<()> {
    let mut chrome_override = None;
    let mut out_override = None;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--chrome" => chrome_override = Some(PathBuf::from(it.next().context(USAGE)?)),
            "--out" => out_override = Some(PathBuf::from(it.next().context(USAGE)?)),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("unexpected argument `{other}`\n{USAGE}"),
        }
    }

    let root = repo_root();
    let out = out_override.unwrap_or_else(|| root.join("dist/demo/printcraft-showcase.pdf"));
    let build = root.join("dist/demo/build");
    fs::create_dir_all(&build).with_context(|| format!("creating {}", build.display()))?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }

    println!("demo-pdf: rendering raster");
    let png = raster::mandelbrot_png(640)?;
    fs::write(build.join("mandelbrot.png"), &png)?;

    let source = root.join("assets/demo/showcase.html");
    let html = fs::read_to_string(&source).with_context(|| format!("reading {}", source.display()))?;
    let html = html
        .replace("{{ASSETS}}", &file_url(&root.join("assets")))
        .replace("{{MANDELBROT_PNG}}", &format!("data:image/png;base64,{}", raster::base64(&png)));
    let html_path = build.join("showcase.html");
    fs::write(&html_path, html)?;

    let chrome = chrome::find(chrome_override.as_deref())?;
    println!("demo-pdf: printing with {}", chrome.display());
    let raw = build.join("chrome.pdf");
    chrome::print_to_pdf(&chrome, &html_path, &raw, &build.join("chrome.log"))?;

    println!("demo-pdf: post-processing");
    let mut doc = lopdf::Document::load(&raw).with_context(|| format!("loading {}", raw.display()))?;
    let summary = postprocess::enhance(&mut doc, pdf::Stamp::now())?;
    doc.compress();
    doc.save(&out).with_context(|| format!("writing {}", out.display()))?;

    println!(
        "demo-pdf: wrote {} ({} pages, {} annotations, {} form fields, outline: {})",
        out.display(),
        summary.pages,
        summary.annotations,
        summary.fields,
        summary.outline
    );
    Ok(())
}

/// The workspace root (the parent of this crate's manifest directory).
fn repo_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = manifest.canonicalize().unwrap_or_else(|_| manifest.to_path_buf());
    manifest.parent().map(Path::to_path_buf).unwrap_or(manifest)
}

/// A `file://` URL for an absolute path, percent-encoding unsafe bytes.
pub(crate) fn file_url(path: &Path) -> String {
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let s = abs.to_string_lossy().replace('\\', "/");
    let mut url = String::from(if s.starts_with('/') { "file://" } else { "file:///" });
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => url.push(byte as char),
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}
