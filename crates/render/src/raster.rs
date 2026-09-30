//! Background page rasterization.
//!
//! A `RenderPool` owns N worker threads. Each worker parses the shared bytes once (hayro's parser
//! is lazy, so this is cheap) and keeps its own render cache. Requests carry a generation number;
//! the UI drops results for stale generations (e.g. after a zoom change).

use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings, render};

/// Hard cap on a rendered side, to bound memory at extreme zoom levels (tiling arrives in M3.3).
const MAX_SIDE: f32 = 8192.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderRequest {
    pub page: usize,
    /// Device pixels per PDF point.
    pub scale: f32,
    /// Caller-defined tag, echoed back (used for zoom generations / thumbnail vs page).
    pub tag: u64,
}

pub struct RenderedPage {
    pub request: RenderRequest,
    pub width: u32,
    pub height: u32,
    /// Premultiplied RGBA8, row-major.
    pub rgba: Vec<u8>,
}

type Queue = Arc<Mutex<Vec<RenderRequest>>>;

pub struct RenderPool {
    queue: Queue,
    wake: Vec<Sender<()>>,
    results: Receiver<RenderedPage>,
    _workers: Vec<JoinHandle<()>>,
}

impl RenderPool {
    pub fn new(bytes: Arc<Vec<u8>>, threads: usize) -> Self {
        let queue: Queue = Arc::new(Mutex::new(Vec::new()));
        let (res_tx, results) = channel();
        let mut wake = Vec::new();
        let mut workers = Vec::new();
        for i in 0..threads.max(1) {
            let (wtx, wrx) = channel::<()>();
            wake.push(wtx);
            let queue = queue.clone();
            let res_tx = res_tx.clone();
            let bytes = bytes.clone();
            let handle = std::thread::Builder::new()
                .name(format!("printcraft-render-{i}"))
                .spawn(move || worker(bytes, queue, wrx, res_tx))
                .expect("spawn render worker");
            workers.push(handle);
        }
        Self { queue, wake, results, _workers: workers }
    }

    /// Replace the pending queue (most urgent first). In-flight renders are not interrupted.
    pub fn set_queue(&self, mut requests: Vec<RenderRequest>) {
        requests.reverse(); // workers pop from the end
        if let Ok(mut q) = self.queue.lock() {
            *q = requests;
        }
        for w in &self.wake {
            let _ = w.send(());
        }
    }

    pub fn try_recv(&self) -> Option<RenderedPage> {
        self.results.try_recv().ok()
    }
}

fn worker(bytes: Arc<Vec<u8>>, queue: Queue, wake: Receiver<()>, out: Sender<RenderedPage>) {
    let Ok(pdf) = Pdf::new(bytes) else { return };
    let settings = InterpreterSettings::default();
    let cache = RenderCache::new();
    let pages = pdf.pages();
    loop {
        let next = queue.lock().ok().and_then(|mut q| q.pop());
        let Some(req) = next else {
            if wake.recv().is_err() {
                return; // pool dropped
            }
            continue;
        };
        let Some(page) = pages.get(req.page) else { continue };
        let (w, h) = page.render_dimensions();
        let scale = req.scale.min(MAX_SIDE / w.max(h).max(1.0));
        let rs = RenderSettings { x_scale: scale, y_scale: scale, bg_color: WHITE, ..Default::default() };
        let pixmap = render(page, &cache, &settings, &rs);
        let (width, height) = (pixmap.width() as u32, pixmap.height() as u32);
        let rgba = pixmap.data_as_u8_slice().to_vec();
        if out.send(RenderedPage { request: req, width, height, rgba }).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE_PAGE: &[u8] = b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 100 50] /Contents 4 0 R >> endobj
4 0 obj << /Length 35 >> stream
0 0 1 rg 10 10 30 20 re f
endstream endobj
trailer << /Root 1 0 R >>
%%EOF";

    #[test]
    fn renders_a_blue_rectangle() {
        let pool = RenderPool::new(Arc::new(ONE_PAGE.to_vec()), 1);
        pool.set_queue(vec![RenderRequest { page: 0, scale: 1.0, tag: 7 }]);
        let page = loop {
            if let Some(p) = pool.try_recv() {
                break p;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!((page.width, page.height, page.request.tag), (100, 50, 7));
        // Pixel (20, 30) in y-down device space is inside the rect drawn at y 10..30 (y-up).
        let px = |x: u32, y: u32| &page.rgba[((y * page.width + x) * 4) as usize..][..4];
        assert_eq!(px(20, 30), &[0, 0, 255, 255]);
        assert_eq!(px(80, 5), &[255, 255, 255, 255]);
    }
}
