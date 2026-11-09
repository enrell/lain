//! Art loading: JPEGs are decoded, cover-cropped to the size they are drawn
//! at (times the display scale) and optionally blurred on GPUI's background
//! executor, then kept as GPU-ready BGRA `RenderImage`s in a cache keyed by
//! (file, size, blur). Pre-sizing matters: GPUI samples images without
//! mipmaps, so drawing a 600x900 poster at 200x300 would shimmer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui::{Context, RenderImage};
use image::imageops::FilterType;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImgKey {
    pub path: PathBuf,
    pub w: u32,
    pub h: u32,
    pub blur: u32,
}

enum Slot {
    Loading,
    Ready(Arc<RenderImage>, Instant),
    Failed,
}

#[derive(Default)]
pub struct ImageStore {
    slots: HashMap<ImgKey, Slot>,
    pub loading: usize,
}

fn decode(key: &ImgKey) -> Result<Arc<RenderImage>, String> {
    let img = image::open(&key.path).map_err(|e| format!("{}: {e}", key.path.display()))?;
    let (w, h) = (key.w.max(1), key.h.max(1));
    let mut img = img.resize_to_fill(w, h, FilterType::CatmullRom);
    if key.blur > 0 {
        // Blur at quarter size: same look, ~16x cheaper.
        let small = img.resize_exact((w / 4).max(1), (h / 4).max(1), FilterType::Triangle);
        img = small.blur(key.blur as f32 / 4.).resize_exact(w, h, FilterType::Triangle);
    }
    let mut buf = img.to_rgba8();
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2); // RGBA -> BGRA, what GPUI's sprite atlas expects
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

impl ImageStore {
    /// The image and when it became ready (for fade-in), or `None` while
    /// it loads (the decode is started on first request).
    pub fn get(&mut self, key: &ImgKey, cx: &mut Context<Self>) -> Option<(Arc<RenderImage>, Instant)> {
        match self.slots.get(key) {
            Some(Slot::Ready(img, at)) => return Some((img.clone(), *at)),
            Some(_) => return None,
            None => {}
        }
        self.slots.insert(key.clone(), Slot::Loading);
        self.loading += 1;
        let k = key.clone();
        let task = cx.background_executor().spawn(async move { decode(&k) });
        let k = key.clone();
        cx.spawn(async move |this, cx| {
            let r = task.await;
            let _ = this.update(cx, |s, cx| {
                s.loading -= 1;
                let slot = match r {
                    Ok(img) => Slot::Ready(img, Instant::now()),
                    Err(e) => {
                        eprintln!("image: {e}");
                        Slot::Failed
                    }
                };
                s.slots.insert(k, slot);
                cx.notify();
            });
        })
        .detach();
        None
    }

    pub fn is_ready(&self, key: &ImgKey) -> bool {
        matches!(self.slots.get(key), Some(Slot::Ready(..)) | Some(Slot::Failed))
    }
}
