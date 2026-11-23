//! Limits for the embedded images of one checked document. A tree owns its cache reservation.

use std::sync::{Arc, Mutex};

/// Limits supplied by the embedder in bytes, pixels and nested SVG documents.
#[derive(Debug, Clone, Copy)]
pub struct SvgImageLimits {
    pub source_bytes: u64,
    pub raster_pixels: u64,
    pub nesting: u32,
}

/// One document's cache allowance after its other assets have been accounted for.
#[derive(Debug, Clone)]
pub struct SvgImageBudget {
    pub(crate) limits: SvgImageLimits,
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    available: u64,
    refusal: Option<&'static str>,
}

pub(crate) const SOURCE: &str = "LIMIT_EXCEEDED:limit.file.bytes: reduce the embedded image source";
pub(crate) const PIXELS: &str = "LIMIT_EXCEEDED:limit.image.pixels: resize the embedded raster";
pub(crate) const NESTING: &str =
    "LIMIT_EXCEEDED:limit.svg.image.nesting: flatten the nested SVG images";
pub(crate) const CACHE: &str = "LIMIT_EXCEEDED:limit.worker.asset-cache.bytes: reduce the embedded images or the other project assets";

impl SvgImageBudget {
    pub fn new(limits: SvgImageLimits, remaining_cache_bytes: u64) -> Self {
        Self {
            limits,
            state: Arc::new(Mutex::new(State {
                available: remaining_cache_bytes,
                refusal: None,
            })),
        }
    }

    /// First refusal in this document, including failed parses that produced no drawable tree.
    pub fn refusal(&self) -> Option<&'static str> {
        self.state.lock().unwrap().refusal
    }

    pub(crate) fn refuse(&self, reason: &'static str) {
        self.state.lock().unwrap().refusal.get_or_insert(reason);
    }

    #[cfg(test)]
    pub(crate) fn available(&self) -> u64 {
        self.state.lock().unwrap().available
    }

    pub(crate) fn reservation(&self) -> Reservation {
        Reservation {
            budget: self.clone(),
            bytes: 0,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Reservation {
    budget: SvgImageBudget,
    bytes: u64,
}

impl Reservation {
    pub(crate) fn add(&mut self, bytes: u64) -> bool {
        let mut state = self.budget.state.lock().unwrap();
        if state.refusal.is_some() {
            return false;
        }
        let Some(left) = state.available.checked_sub(bytes) else {
            state.refusal = Some(CACHE);
            return false;
        };
        state.available = left;
        self.bytes += bytes;
        true
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.state.lock().unwrap().available += self.bytes;
    }
}

/// Per-parse state. The parser invokes the data resolver recursively on this thread.
/// Separate roots share the document budget, with a separate nesting depth for each root.
pub(crate) struct CheckedParse<'a> {
    budget: &'a SvgImageBudget,
    held: Mutex<Reservation>,
    depth: std::sync::atomic::AtomicU32,
}

impl<'a> CheckedParse<'a> {
    pub(crate) fn new(budget: &'a SvgImageBudget) -> Self {
        Self {
            budget,
            held: Mutex::new(budget.reservation()),
            depth: 0.into(),
        }
    }

    pub(crate) fn into_reservation(self) -> Arc<Reservation> {
        Arc::new(self.held.into_inner().unwrap())
    }

    pub(crate) fn source_fits(&self, bytes: &[u8]) -> bool {
        if bytes.len() as u64 > self.budget.limits.source_bytes {
            self.budget.refuse(SOURCE);
            return false;
        }
        self.budget.refusal().is_none()
    }

    pub(crate) fn charge_source(&self, data: &[u8]) -> bool {
        self.source_fits(data) && self.held.lock().unwrap().add(data.len() as u64)
    }

    pub(crate) fn charge_raster(&self, data: &[u8]) -> Option<()> {
        // The same header reader as usvg's ImageKind::actual_size, before any pixel decode.
        let size = imagesize::blob_size(data).ok()?;
        let pixels = (size.width as u64).checked_mul(size.height as u64);
        let Some(pixels) = pixels.filter(|pixels| *pixels <= self.budget.limits.raster_pixels)
        else {
            self.budget.refuse(PIXELS);
            return None;
        };
        let Some(bytes) = pixels.checked_mul(4) else {
            self.budget.refuse(CACHE);
            return None;
        };
        self.held.lock().unwrap().add(bytes).then_some(())
    }

    pub(crate) fn enter_svg(&self) -> Option<NestedSvg<'_>> {
        use std::sync::atomic::Ordering;
        if self
            .depth
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |depth| {
                (depth < self.budget.limits.nesting).then(|| depth + 1)
            })
            .is_err()
        {
            self.budget.refuse(NESTING);
            return None;
        }
        Some(NestedSvg(&self.depth))
    }

    /// Inflate SVGZ incrementally. Each part is admitted before growing the buffer;
    /// the caller hands plain XML to both the refusal scanner and the SVG parser.
    pub(crate) fn xml<'b>(
        &self,
        data: &'b [u8],
    ) -> Result<std::borrow::Cow<'b, [u8]>, usvg::Error> {
        use std::borrow::Cow;
        use std::io::Read;
        if !self.source_fits(data) {
            return Err(usvg::Error::ElementsLimitReached);
        }
        if !data.starts_with(&[0x1f, 0x8b]) {
            return Ok(Cow::Borrowed(data));
        }
        let mut decoder = flate2::read::GzDecoder::new(data);
        let mut output = Vec::new();
        let mut chunk = [0; 8192];
        loop {
            let n = decoder
                .read(&mut chunk)
                .map_err(|_| usvg::Error::MalformedGZip)?;
            if n == 0 {
                break;
            }
            if (output.len() as u64).saturating_add(n as u64) > self.budget.limits.source_bytes {
                self.budget.refuse(SOURCE);
                return Err(usvg::Error::ElementsLimitReached);
            }
            if !self.held.lock().unwrap().add(n as u64) {
                return Err(usvg::Error::ElementsLimitReached);
            }
            output.extend_from_slice(&chunk[..n]);
        }
        std::str::from_utf8(&output).map_err(|_| usvg::Error::NotAnUtf8Str)?;
        Ok(Cow::Owned(output))
    }
}

pub(crate) struct NestedSvg<'a>(&'a std::sync::atomic::AtomicU32);

impl Drop for NestedSvg<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests;
