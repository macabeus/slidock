//! An oracle PDF as the bench uses it: a copy under `<cache>/oracle/src/<sha>.pdf` (the renderer mounts only the cache,
//! and every tool opens the copy), and its page sizes from poppler's `pdfinfo`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cache::{read_json, temp_sibling, write_json};
use crate::error::{BenchError, Result};
use crate::renderer::docker::Renderer;

/// A page size in points.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageSize {
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Debug)]
pub struct OraclePdf {
    pub sha256: String,
    /// The copy on the host, under the renderer's /work.
    pub path: PathBuf,
    /// The copy's path inside the renderer.
    pub container_path: String,
}

/// Copies a PDF into the oracle sources by its sha256 (once). `work` is the folder mounted at /work.
pub fn oracle_copy(work: &Path, file: &Path, sha256: &str) -> Result<OraclePdf> {
    let dir = work.join("src");
    let path = dir.join(format!("{sha256}.pdf"));
    if !path.exists() {
        std::fs::create_dir_all(&dir).map_err(BenchError::io(&dir))?;
        let temp = temp_sibling(&path);
        std::fs::copy(file, &temp).map_err(BenchError::io(file))?;
        std::fs::rename(&temp, &path).map_err(BenchError::io(&path))?;
    }
    Ok(OraclePdf {
        sha256: sha256.to_owned(),
        path,
        container_path: format!("/work/src/{sha256}.pdf"),
    })
}

/// The pages of `pdfinfo -f 1 -l 100000` output: `Page <n> size: <w> x <h> pts`.
pub fn parse_page_sizes(output: &str) -> Vec<PageSize> {
    let mut sizes: Vec<Option<PageSize>> = Vec::new();
    for line in output.lines() {
        let Some(rest) = line.strip_prefix("Page") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some((number, after)) = rest.split_once(" size:") else {
            continue;
        };
        let Ok(number) = number.parse::<usize>() else {
            continue;
        };
        let mut parts = after.split_whitespace();
        let (Some(w), Some("x"), Some(h), Some(unit)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let number_like =
            |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'.');
        if !unit.starts_with("pts") || !number_like(w) || !number_like(h) || number == 0 {
            continue;
        }
        if sizes.len() < number {
            sizes.resize(number, None);
        }
        if let Some(slot) = sizes.get_mut(number - 1) {
            *slot = Some(PageSize {
                w: slidock_canon::js_number(w),
                h: slidock_canon::js_number(h),
            });
        }
    }
    sizes.into_iter().flatten().collect()
}

/// Every page's size in points (cached beside the copy).
pub async fn page_sizes(renderer: &Renderer, pdf: &OraclePdf) -> Result<Vec<PageSize>> {
    let cached = PathBuf::from(format!("{}.pages.json", pdf.path.display()));
    if let Some(known) = read_json::<Vec<PageSize>>(&cached)? {
        return Ok(known);
    }
    let result = renderer
        .exec(
            &["pdfinfo", "-f", "1", "-l", "100000", &pdf.container_path],
            Duration::from_secs(60),
        )
        .await?;
    if !result.ok() {
        return Err(BenchError::failed(format!(
            "pdfinfo failed on {}: {}",
            pdf.sha256,
            result.stderr.trim()
        )));
    }
    let sizes = parse_page_sizes(&result.stdout_text());
    write_json(&cached, &sizes)?;
    Ok(sizes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_page_sizes() {
        let output = "Producer: x\nPage    1 size: 960 x 540 pts\nPage    2 size: 720.5 x 405 pts (A4)\nPages: 2\n";
        assert_eq!(
            parse_page_sizes(output),
            [
                PageSize { w: 960.0, h: 540.0 },
                PageSize { w: 720.5, h: 405.0 }
            ]
        );
    }
}
