//! Art loading. Images come from the server (`/api/items/{id}/thumbnail`,
//! avatars) or from local files (theme wallpapers). They are decoded,
//! cover-cropped to the size they are drawn at (times the display scale)
//! and optionally blurred off the UI thread, then kept as GPU-ready BGRA
//! `RenderImage`s keyed by (source, size, blur). Pre-sizing matters: GPUI
//! samples images without mipmaps, so a big poster drawn small shimmers.
//!
//! A server that does not serve thumbnails yet (501 `not-ported`) or has
//! the watch module off is remembered, so cards fall back to typographic
//! placeholders without a request per card.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gpui::{Context, RenderImage};
use image::imageops::FilterType;
use lain_client::ClientError;

use crate::backend::Backend;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    Thumb(String),
    Avatar(String, i64),
    /// A comic page: item id, page index.
    Page(String, i64),
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImgKey {
    pub src: Source,
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
    raw: HashMap<Source, Arc<Vec<u8>>>,
    pub loading: usize,
    /// Thumbnails are not served (not ported / module off): skip them.
    pub thumbs_off: bool,
    pub backend: Option<Arc<Backend>>,
}

fn decode(bytes: &[u8], w: u32, h: u32, blur: u32) -> Result<Arc<RenderImage>, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let (w, h) = (w.max(1), h.max(1));
    let mut img = img.resize_to_fill(w, h, FilterType::CatmullRom);
    if blur > 0 {
        // Blur at quarter size: same look, ~16x cheaper.
        let small = img.resize_exact((w / 4).max(1), (h / 4).max(1), FilterType::Triangle);
        img = small.blur(blur as f32 / 4.).resize_exact(w, h, FilterType::Triangle);
    }
    let mut buf = img.to_rgba8();
    for px in buf.as_chunks_mut::<4>().0 {
        px.swap(0, 2); // RGBA -> BGRA, what GPUI's sprite atlas expects
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

impl ImageStore {
    /// Forget everything (server switch, theme reload of local art).
    pub fn reset(&mut self, backend: Option<Arc<Backend>>) {
        self.slots.clear();
        self.raw.clear();
        self.thumbs_off = false;
        self.backend = backend;
    }

    /// Drop cached avatars (after an upload).
    pub fn forget(&mut self, src: &Source) {
        self.raw.remove(src);
        self.slots.retain(|k, _| &k.src != src);
    }

    /// The image and when it became ready (for fade-in), or `None` while
    /// it loads or when there is none (the decode starts on first ask).
    pub fn get(&mut self, key: &ImgKey, cx: &mut Context<Self>) -> Option<(Arc<RenderImage>, Instant)> {
        match self.slots.get(key) {
            Some(Slot::Ready(img, at)) => return Some((img.clone(), *at)),
            Some(_) => return None,
            None => {}
        }
        if matches!(key.src, Source::Thumb(_)) && self.thumbs_off {
            return None;
        }
        self.slots.insert(key.clone(), Slot::Loading);
        self.loading += 1;
        let k = key.clone();
        let cached = self.raw.get(&key.src).cloned();
        let backend = self.backend.clone();
        let fetch = match (&cached, &key.src, &backend) {
            (Some(_), _, _) | (None, Source::File(_), _) => None,
            (None, Source::Thumb(id), Some(b)) => {
                let (c, id) = (b.client.clone(), id.clone());
                Some(b.spawn(async move { c.thumbnail(&id).await }))
            }
            (None, Source::Page(id, n), Some(b)) => {
                let (c, id, n) = (b.client.clone(), id.clone(), *n);
                Some(b.spawn(async move { c.comic_page(&id, n).await }))
            }
            (None, Source::Avatar(id, _), Some(b)) => {
                let (c, id) = (b.client.clone(), id.clone());
                Some(b.spawn(async move { c.avatar(&id).await }))
            }
            (None, _, None) => None,
        };
        cx.spawn(async move |this, cx| {
            let bytes: Result<Arc<Vec<u8>>, (String, bool)> = match (cached, fetch, &k.src) {
                (Some(b), _, _) => Ok(b),
                (None, _, Source::File(p)) => {
                    std::fs::read(p).map(Arc::new).map_err(|e| (format!("{}: {e}", p.display()), false))
                }
                (None, Some(task), _) => match task.await {
                    Ok(Ok(b)) => Ok(Arc::new(b)),
                    Ok(Err(e)) => {
                        let off = matches!(e, ClientError::NotPorted { .. } | ClientError::ModuleDisabled { .. });
                        Err((e.to_string(), off))
                    }
                    Err(e) => Err((e.to_string(), false)),
                },
                (None, None, _) => Err(("no server".into(), false)),
            };
            let decoded = match &bytes {
                Ok(b) => {
                    let (b, w, h, blur) = (b.clone(), k.w, k.h, k.blur);
                    Some(cx.background_executor().spawn(async move { decode(&b, w, h, blur) }).await)
                }
                Err(_) => None,
            };
            let _ = this.update(cx, |s, cx| {
                s.loading = s.loading.saturating_sub(1);
                let slot = match (bytes, decoded) {
                    (Ok(raw), Some(Ok(img))) => {
                        s.raw.insert(k.src.clone(), raw);
                        Slot::Ready(img, Instant::now())
                    }
                    (Ok(_), Some(Err(e))) => {
                        eprintln!("image: {e}");
                        Slot::Failed
                    }
                    (Err((e, off)), _) => {
                        if off && matches!(k.src, Source::Thumb(_)) {
                            if !s.thumbs_off {
                                eprintln!("thumbnails: {e}; using placeholders");
                            }
                            s.thumbs_off = true;
                        }
                        Slot::Failed
                    }
                    _ => Slot::Failed,
                };
                s.slots.insert(k, slot);
                cx.notify();
            });
        })
        .detach();
        None
    }
}
