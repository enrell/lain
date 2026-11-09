//! Off-thread art decoding: a few worker threads decode, crop-resize (and
//! blur, for the hero) the local JPEGs into pixel buffers; the UI thread
//! only wraps the finished buffers into `slint::Image`s. Each series' art
//! is decoded once and kept in the UI models, which act as the cache.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use image::imageops::FilterType;
use slint::{Rgb8Pixel, SharedPixelBuffer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Poster,
    Thumb,
    Hero,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub kind: Kind,
    pub series: usize,
    pub path: PathBuf,
    /// target size in physical pixels (crop to fill)
    pub w: u32,
    pub h: u32,
    pub blur: f32,
    /// bumps when the theme changes; stale results are dropped
    pub generation: u64,
}

pub struct Done {
    pub kind: Kind,
    pub series: usize,
    pub generation: u64,
    pub buf: SharedPixelBuffer<Rgb8Pixel>,
    pub ms: f64,
}

pub struct Loader {
    queue: Arc<(Mutex<VecDeque<Job>>, std::sync::Condvar)>,
}

impl Loader {
    pub fn new(workers: usize, done: impl Fn(Done) + Send + Sync + 'static) -> Loader {
        let queue: Arc<(Mutex<VecDeque<Job>>, std::sync::Condvar)> = Arc::default();
        let done = Arc::new(done);
        for i in 0..workers {
            let q = queue.clone();
            let done = done.clone();
            std::thread::Builder::new()
                .name(format!("art-{i}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let (lock, cv) = &*q;
                            let mut g = lock.lock().unwrap();
                            loop {
                                if let Some(j) = g.pop_front() {
                                    break j;
                                }
                                g = cv.wait(g).unwrap();
                            }
                        };
                        let t = std::time::Instant::now();
                        match decode(&job) {
                            Ok(buf) => done(Done {
                                kind: job.kind,
                                series: job.series,
                                generation: job.generation,
                                buf,
                                ms: t.elapsed().as_secs_f64() * 1000.0,
                            }),
                            Err(e) => eprintln!("art {}: {e}", job.path.display()),
                        }
                    }
                })
                .expect("spawn art worker");
        }
        Loader { queue }
    }

    /// Queues jobs; `front` puts them before what is already waiting.
    pub fn push(&self, jobs: Vec<Job>, front: bool) {
        let (lock, cv) = &*self.queue;
        let mut g = lock.lock().unwrap();
        if front {
            for j in jobs.into_iter().rev() {
                g.push_front(j);
            }
        } else {
            g.extend(jobs);
        }
        cv.notify_all();
    }

    pub fn clear(&self) {
        self.queue.0.lock().unwrap().clear();
    }
}

fn decode(job: &Job) -> Result<SharedPixelBuffer<Rgb8Pixel>, String> {
    let img = image::open(&job.path).map_err(|e| e.to_string())?;
    let (sw, sh) = (img.width() as f32, img.height() as f32);
    let (tw, th) = (job.w.max(1), job.h.max(1));
    // crop to the target aspect ratio, then resize
    let scale = (tw as f32 / sw).max(th as f32 / sh);
    let (cw, ch) = ((tw as f32 / scale).round() as u32, (th as f32 / scale).round() as u32);
    let (cx, cy) = (((sw as u32).saturating_sub(cw)) / 2, ((sh as u32).saturating_sub(ch)) / 2);
    let mut img = img.crop_imm(cx, cy, cw.min(sw as u32), ch.min(sh as u32));
    if img.width() != tw || img.height() != th {
        img = img.resize_exact(tw, th, if scale < 1.0 { FilterType::CatmullRom } else { FilterType::Triangle });
    }
    if job.blur > 0.0 {
        img = img.fast_blur(job.blur);
    }
    let rgb = img.into_rgb8();
    Ok(SharedPixelBuffer::clone_from_slice(rgb.as_raw(), rgb.width(), rgb.height()))
}
