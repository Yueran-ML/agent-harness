//! Pipelines made on threads of their own, so that a frame does not wait for its shaders.
//!
//! Making a pipeline compiles its shaders, and FXC (the compiler of Direct3D 12 that the engine
//! uses, see `gpu.rs`) takes from a tenth of a second for a small program of colour to a few
//! seconds for a grade of many nodes. A [`Builder`] makes pipelines on a few threads of its own,
//! as many at once as it has threads: a compositor asks it for those a frame needs and is not
//! held up by them ([`crate::Compositor::compile_in_background`]), or waits for all of them at
//! once, made side by side, which is still sooner than one after another.
//!
//! What a pipeline is made of is given as a function of the device ([`Make`]), which the thread
//! that makes it calls. It catches the errors of validation of the pipeline it makes in an error
//! scope of its own: the scopes of wgpu are of a thread.

use std::collections::HashSet;
use std::hash::Hash;
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::gpu::Gpu;

/// The most threads that a builder makes pipelines on.
const MOST_THREADS: usize = 4;

/// What makes a pipeline on the device: the pipeline, or why it cannot be made.
pub(crate) type Make = Box<dyn FnOnce(&wgpu::Device) -> Made + Send>;

/// A pipeline made, or why it was not.
pub(crate) type Made = Result<wgpu::RenderPipeline, String>;

/// What is called once a pipeline asked for is made, on the thread that made it.
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

struct Job<K> {
    key: K,
    make: Make,
}

/// What the threads hand back, and how many jobs are not done.
struct Done<K> {
    made: Vec<(K, Made)>,
    unfinished: usize,
}

struct Shared<K> {
    done: Mutex<Done<K>>,
    changed: Condvar,
    wake: Mutex<Option<Wake>>,
}

impl<K> Shared<K> {
    fn lock(&self) -> MutexGuard<'_, Done<K>> {
        self.done.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Makes pipelines on threads of its own: those of one card, each known by a key.
pub(crate) struct Builder<K> {
    gpu: Arc<Gpu>,
    shared: Arc<Shared<K>>,
    sender: Option<mpsc::Sender<Job<K>>>,
    receiver: Arc<Mutex<mpsc::Receiver<Job<K>>>>,
    threads: usize,
    /// The keys asked for and not yet taken back.
    pending: HashSet<K>,
}

impl<K: Clone + Eq + Hash + Send + 'static> Builder<K> {
    pub(crate) fn new(gpu: Arc<Gpu>) -> Builder<K> {
        let (sender, receiver) = mpsc::channel();
        Builder {
            gpu,
            shared: Arc::new(Shared {
                done: Mutex::new(Done {
                    made: Vec::new(),
                    unfinished: 0,
                }),
                changed: Condvar::new(),
                wake: Mutex::new(None),
            }),
            sender: Some(sender),
            receiver: Arc::new(Mutex::new(receiver)),
            threads: 0,
            pending: HashSet::new(),
        }
    }

    /// Call `wake` each time a pipeline is made from now on.
    pub(crate) fn wake_with(&mut self, wake: Wake) {
        *self
            .shared
            .wake
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(wake);
    }

    /// Whether the pipeline of `key` is being made.
    pub(crate) fn is_pending(&self, key: &K) -> bool {
        self.pending.contains(key)
    }

    /// Whether any pipeline is being made: not one that is made, and not taken yet.
    pub(crate) fn is_busy(&self) -> bool {
        self.shared.lock().unfinished > 0
    }

    /// Make the pipeline of `key` with `make`, unless it is being made already.
    pub(crate) fn ask(&mut self, key: K, make: Make) {
        if !self.pending.insert(key.clone()) {
            return;
        }
        let wanted = self.pending.len().min(MOST_THREADS);
        while self.threads < wanted && self.spawn() {
            self.threads += 1;
        }
        let job = Job {
            key: key.clone(),
            make,
        };
        // Counted before it is sent, so that the thread that makes it counts it off after.
        let mut sent = false;
        if self.threads > 0 {
            self.shared.lock().unfinished += 1;
            sent = self
                .sender
                .as_ref()
                .is_some_and(|sender| sender.send(job).is_ok());
        }
        let mut done = self.shared.lock();
        if self.threads > 0 && !sent {
            done.unfinished -= 1;
        }
        if !sent {
            done.made.push((
                key,
                Err("no thread could be started to make the pipeline".into()),
            ));
        }
    }

    /// Start a thread that makes pipelines until the builder is dropped.
    fn spawn(&self) -> bool {
        let receiver = Arc::clone(&self.receiver);
        let shared = Arc::clone(&self.shared);
        let gpu = Arc::clone(&self.gpu);
        std::thread::Builder::new()
            .name("columbina-pipelines".into())
            .spawn(move || loop {
                let job = {
                    let receiver = receiver.lock().unwrap_or_else(PoisonError::into_inner);
                    receiver.recv()
                };
                let Ok(Job { key, make }) = job else { return };
                let made = make(gpu.device());
                {
                    let mut done = shared.lock();
                    done.unfinished -= 1;
                    done.made.push((key, made));
                }
                shared.changed.notify_all();
                let wake = shared
                    .wake
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                if let Some(wake) = wake {
                    wake();
                }
            })
            .is_ok()
    }

    /// The pipelines made since this was last asked, at once.
    pub(crate) fn take(&mut self) -> Vec<(K, Made)> {
        let made = std::mem::take(&mut self.shared.lock().made);
        for (key, _) in &made {
            self.pending.remove(key);
        }
        made
    }

    /// Wait until every pipeline asked for is made, and give those not taken yet.
    pub(crate) fn finish(&mut self) -> Vec<(K, Made)> {
        let mut done = self.shared.lock();
        while done.unfinished > 0 {
            done = self
                .shared
                .changed
                .wait(done)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(done);
        self.take()
    }
}

impl<K> Drop for Builder<K> {
    fn drop(&mut self) {
        // The threads end once the jobs sent are done: none is waited for.
        self.sender = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    const SHADER: &str =
        "@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }\n\
                          @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(K); }\n";

    fn make(source: String) -> Make {
        Box::new(move |device: &wgpu::Device| {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                }),
                multiview_mask: None,
                cache: None,
            });
            match pollster::block_on(scope.pop()) {
                Some(error) => Err(error.to_string()),
                None => Ok(pipeline),
            }
        })
    }

    #[test]
    fn pipelines_are_made_on_threads_of_their_own_and_each_wakes_who_asked() {
        let Some(gpu) = crate::gpu_for_tests() else {
            return;
        };
        let woken = Arc::new(AtomicUsize::new(0));
        let mut builder: Builder<u32> = Builder::new(gpu);
        builder.wake_with({
            let woken = Arc::clone(&woken);
            Arc::new(move || {
                woken.fetch_add(1, Ordering::SeqCst);
            })
        });
        for key in 0..6_u32 {
            builder.ask(key, make(SHADER.replace('K', &format!("{key}.0"))));
        }
        // Asked again while it is being made: made once.
        builder.ask(0, make(SHADER.replace('K', "0.0")));
        assert!(builder.is_pending(&3));
        let mut made = builder.finish();
        made.sort_by_key(|(key, _)| *key);
        assert_eq!(
            made.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            (0..6).collect::<Vec<_>>()
        );
        assert!(made.iter().all(|(_, made)| made.is_ok()));
        assert!(!builder.is_busy());
        assert_eq!(woken.load(Ordering::SeqCst), 6);
        // A shader that is not WGSL is an error of its own pipeline, and of no other.
        builder.ask(7, make(SHADER.replace('K', "nothing")));
        builder.ask(8, make(SHADER.replace('K', "1.0")));
        let started = Instant::now();
        let mut made = Vec::new();
        while made.len() < 2 && started.elapsed() < Duration::from_secs(60) {
            made.extend(builder.take());
            std::thread::yield_now();
        }
        made.sort_by_key(|(key, _)| *key);
        assert!(made[0].1.is_err(), "{:?}", made[0].1.as_ref().err());
        assert!(made[1].1.is_ok());
    }
}
