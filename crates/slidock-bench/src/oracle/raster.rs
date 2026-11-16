//! Oracle page rasters: `pdftoppm` inside the renderer container (`renderer::docker`) at the size the metrics score
//! at (`slidock_bench_metrics::raster_size`), cached under `raster/<key>/<pdf sha256>/` by a key that names everything
//! that shapes the pixels (size, poppler, the renderer platform and this module's source). `oracle --record` pins each
//! page's sha256 in the corpus manifest under that key, and `oracle::resolve` checks every raster against it.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::error::{BenchError, Result};
use crate::hash::{sha256, sha256_json};
use crate::oracle::pdf::{OraclePdf, PageSize};
use crate::renderer::docker::{PLATFORM_DIGEST, Renderer};

/// This module's source: an edited extractor reads only its own output.
const CODE: &str = include_str!("raster.rs");

/// The pixel size of a page or slide of `size` points: 1920 px on the long side.
pub fn raster_size(size: PageSize) -> (u32, u32) {
    let (width, height, _) = slidock_bench_metrics::raster_size(size.w, size.h);
    (width as u32, height as u32)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RasterKey<'a> {
    width: u32,
    height: u32,
    poppler: &'a str,
    platform_digest: &'a str,
    code: String,
}

pub fn raster_key(size: (u32, u32), poppler: &str) -> Result<String> {
    sha256_json(&RasterKey {
        width: size.0,
        height: size.1,
        poppler,
        platform_digest: PLATFORM_DIGEST,
        code: sha256(CODE),
    })
}

#[derive(Clone, Debug)]
pub struct PageRaster {
    pub path: PathBuf,
    pub sha256: String,
}

/// The PNG of one page (1-based), rasterised once per key.
pub async fn raster_page(
    renderer: &Renderer,
    pdf: &OraclePdf,
    page: u32,
    size: (u32, u32),
    key: &str,
    no_cache: bool,
) -> Result<PageRaster> {
    let relative = format!("raster/{key}/{}", pdf.sha256);
    let dir = renderer.work.join(&relative);
    let path = dir.join(format!("{page}.png"));
    if no_cache || !path.exists() {
        std::fs::create_dir_all(&dir).map_err(BenchError::io(&dir))?;
        let prefix = format!("{page}.{}.tmp", std::process::id());
        let page_text = page.to_string();
        let (width, height) = (size.0.to_string(), size.1.to_string());
        let out = format!("/work/{relative}/{prefix}");
        let result = renderer
            .exec(
                &[
                    "pdftoppm",
                    "-png",
                    "-singlefile",
                    "-f",
                    &page_text,
                    "-l",
                    &page_text,
                    "-scale-to-x",
                    &width,
                    "-scale-to-y",
                    &height,
                    &pdf.container_path,
                    &out,
                ],
                Duration::from_secs(60),
            )
            .await?;
        let temp = dir.join(format!("{prefix}.png"));
        if !result.ok() || !temp.exists() {
            let _ = std::fs::remove_file(&temp);
            let why = if result.timed_out {
                "timed out".to_owned()
            } else {
                result.stderr.trim().to_owned()
            };
            return Err(BenchError::failed(format!(
                "pdftoppm page {page} of {}: {why}",
                pdf.sha256
            )));
        }
        std::fs::rename(&temp, &path).map_err(BenchError::io(&path))?;
    }
    let bytes = std::fs::read(&path).map_err(BenchError::io(&path))?;
    Ok(PageRaster {
        path,
        sha256: sha256(bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasters_put_1920_px_on_the_long_side() {
        assert_eq!(raster_size(PageSize { w: 960.0, h: 540.0 }), (1920, 1080));
        assert_eq!(raster_size(PageSize { w: 540.0, h: 960.0 }), (1080, 1920));
        assert_eq!(raster_size(PageSize { w: 720.0, h: 405.0 }), (1920, 1080));
    }

    #[test]
    fn the_key_names_size_poppler_and_code() {
        let a = raster_key((1920, 1080), "24.02.0").unwrap();
        assert_ne!(a, raster_key((1920, 1081), "24.02.0").unwrap());
        assert_ne!(a, raster_key((1920, 1080), "24.03.0").unwrap());
    }
}
