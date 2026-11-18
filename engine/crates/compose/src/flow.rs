//! The flows between two pictures of one media, on the card: how far each point of the first
//! moves to be in the second (forward), and each point of the second to be in the first
//! (backward). A clip that is retimed by Optical Flow
//! ([`columbina_model::retime::RetimeProcess::OpticalFlow`]) shows at a frame between two
//! pictures each of them moved along these to where it would be at the frame, and mixed then
//! ([`crate::Source::Between`], `flow_between.wgsl`).
//!
//! They are found by Dense Inverse Search (Kroeger, Timofte, Dai and Van Gool, "Fast Optical
//! Flow using Dense Inverse Search", ECCV 2016), on the luma of the two pictures, by the passes of
//! `flow.wgsl`, none of which is read back:
//!
//! - **A pyramid.** The finest level is the luma of each picture averaged over blocks of `scale`
//!   × `scale` of its pixels, `scale` the least power of two that makes its long side no more
//!   than [`FINEST`]: the pictures themselves up to 2K, halved up to 4K, and a quarter of 8K.
//!   Each level after it is the one before it halved, by the filter `[1 3 3 1] / 8` across and
//!   down, until the long side of a level is below [`COARSEST`], so from 32 to 63 pixels, or
//!   there are [`MOST_LEVELS`].
//! - **Patches.** From the coarsest level to the finest, a level is cut into patches of 8 × 8 of
//!   its pixels, one every 4 pixels across and down, the last of each row and each column laid
//!   against the far edge. Each patch begins where the flows of the level coarser than it put
//!   its middle (at the coarsest, where it is), and is moved by the inverse compositional
//!   search of Gauss–Newton of the paper, [`ITERATIONS`] steps at most: the patch of one picture
//!   against the other picture, each less its mean over the patch, so that a change of
//!   brightness alone moves nothing. Each step goes a pixel of the level at most; a patch that
//!   goes more than 4 pixels from where it began, or is less alike the other picture where it
//!   ends than where it began, stays where it began.
//! - **Dense flows.** Each pixel of the level takes the flows of the patches that lie over it,
//!   each weighed by `1 / max(1, d)` of the difference `d` of the two pictures under it there, in
//!   codes of 8 bits, as the paper weighs them. The variational refinement of the paper is left
//!   out: the flows of the finest level are smoothed once instead, over the three by three
//!   pixels about each, by those of a luma near its own.
//!
//! Both flows are found at once, level by level. Those of the finest level are written into a
//! texture of half floats of its size, forward in red and green and backward in blue and alpha,
//! in pixels of the pictures, which the layer reads (`flow_between.wgsl`).
//!
//! The compositor keeps the flows of the last few pairs of pictures ([`KEPT`]), each known by
//! its [`Pair`] and the size of its pictures, so that the several frames that lie between the
//! same two pictures, as those of a clip at a quarter of its speed, find them once. What the
//! passes work in (the pyramid, the flows of the patches and of two levels) is kept by the size
//! of the finest level, and let go of where no flows of that size are found for a while, as
//! are flows that no frame reads for a while ([`KEPT_FOR`] frames). Flows that the card cannot
//! hold, as where what they work in is larger than its largest buffer, are not found: the layer
//! mixes its two pictures as Frame Blend does.

use std::collections::hash_map::Entry;
use std::collections::HashMap;

use crate::gpu::Gpu;
use crate::yuv::Texel;

/// The shader of the passes.
const SHADER: &str = include_str!("flow.wgsl");

/// The long side of the finest level, at most, in its pixels.
pub const FINEST: u32 = 2048;

/// A level whose long side is this or more is halved again, so that the long side of the
/// coarsest is from half this to one pixel less than this (or that of the finest, where it is
/// already less).
pub const COARSEST: u32 = 64;

/// The most levels of a pyramid.
pub const MOST_LEVELS: usize = 8;

/// The most steps of the search of a patch.
pub const ITERATIONS: u16 = 16;

/// How many pairs of pictures the flows of are kept, the last read.
pub const KEPT: usize = 3;

/// How many frames flows, and what the passes work in, are kept for while no frame reads them.
pub const KEPT_FOR: u64 = 300;

/// The format of the texture of flows.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The pixels of a patch across and down, and how far apart patches begin, as `flow.wgsl` has
/// them.
const PATCH: u32 = 8;
const STRIDE: u32 = 4;

/// The invocations of a group of the passes across and down.
const GROUP: u32 = 8;

/// The bytes of the uniform of `flow.wgsl` (`Params`).
const PARAMS_SIZE: u64 = 112;

/// Where the first part of the uniform of `yuv.wgsl` of a picture
/// ([`crate::yuv::picture_params`]) holds what a texel of its plane of luma is multiplied by to
/// be a code, the code of black, the range of codes to white, how the picture is stored (0
/// Y'CbCr, 1 R'G'B'A of integers, 2 of floats) and what a texel of R'G'B'A is multiplied by to be
/// from 0 to 1.
const SCALE_PARAM: usize = 8;
const BLACK_PARAM: usize = 9;
const RANGE_PARAM: usize = 10;
const KIND_PARAM: usize = 24;
const UNIT_PARAM: usize = 26;

/// What the two pictures of a layer that is retimed by Optical Flow are known by, for the flows
/// between them to be kept ([`crate::Source::Between`]): the file that they are read from, as
/// whoever reads them names it (the engine names the media of the clip, and whether its
/// original or its proxy is read, and which), and the frame of each in it. The flows of a pair
/// are kept for pictures of one size: those of the same pair at another size are others.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pair {
    /// The file that the two pictures are read from.
    pub media: String,
    /// The frame of the first picture in it.
    pub first: i64,
    /// The frame of the second.
    pub second: i64,
}

/// A level of a pyramid: its width and height in its pixels, where its luma begins among those
/// of all the levels (in pairs of the two pictures), and its patches across and down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Level {
    width: u32,
    height: u32,
    luma: u32,
    across: u32,
    down: u32,
}

/// How many patches lie along a row (or a column) of `size` pixels.
fn patches(size: u32) -> u32 {
    (size.max(PATCH) - PATCH).div_ceil(STRIDE) + 1
}

/// The levels of the pyramid of pictures of `width` × `height`, the finest first, and how many
/// pixels of the pictures a pixel of the finest is, across and down.
fn levels(width: u32, height: u32) -> (u16, Vec<Level>) {
    let mut scale: u16 = 1;
    while width.max(height).div_ceil(u32::from(scale)) > FINEST {
        scale *= 2;
    }
    let (mut across, mut down) = (
        width.div_ceil(u32::from(scale)).max(1),
        height.div_ceil(u32::from(scale)).max(1),
    );
    let mut levels = Vec::new();
    let mut luma = 0;
    loop {
        levels.push(Level {
            width: across,
            height: down,
            luma,
            across: patches(across),
            down: patches(down),
        });
        luma += across * down;
        if across.max(down) < COARSEST || levels.len() == MOST_LEVELS {
            return (scale, levels);
        }
        (across, down) = (across.div_ceil(2), down.div_ceil(2));
    }
}

/// The pictures that flows are found between: the planes of luma of the two (of R'G'B'A, their
/// one planes) as the layer has them, read as `texel`s, of `size`, and the first part of the
/// uniform of `yuv.wgsl` of either ([`crate::yuv::picture_params`]), which says how their codes
/// are read.
pub(crate) struct Pictures<'a> {
    pub(crate) first: &'a wgpu::Texture,
    pub(crate) second: &'a wgpu::Texture,
    pub(crate) texel: Texel,
    pub(crate) size: (u32, u32),
    pub(crate) decoded: &'a [f32],
}

/// The flows of a pair of pictures: the view of their texture, and how many pixels of the
/// pictures a texel of it is, across and down.
pub(crate) struct Flows {
    pub(crate) view: wgpu::TextureView,
    pub(crate) scale: u16,
}

/// The flows of a pair of pictures kept.
struct Kept {
    pair: Pair,
    /// The size of the pictures.
    size: (u32, u32),
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    scale: u16,
    /// The last frame that read them.
    used: u64,
    /// The last frame that asked a job to find them.
    asked: u64,
    /// Whether a job found them: one that was done, not only asked for.
    found: bool,
    /// What the job that finds them knows them by.
    id: u64,
}

/// What the passes of flows whose finest level is of one size work in: the pyramid, the flows
/// of the patches of a level, and those of two levels, one after the other (`even` those of the
/// finest level and every second one after, `odd` those of the others).
struct Work {
    luma: wgpu::Buffer,
    even: wgpu::Buffer,
    odd: wgpu::Buffer,
    patches: wgpu::Buffer,
    /// The last frame that found flows in it.
    used: u64,
}

impl Work {
    /// What the passes of the pyramid of `levels` work in; none where a buffer of it would be
    /// larger than the card holds, or binds at once.
    fn new(device: &wgpu::Device, levels: &[Level]) -> Option<Work> {
        let pixels = |level: &Level| u64::from(level.width) * u64::from(level.height);
        let finest = levels.first()?;
        let luma = levels.iter().map(pixels).sum::<u64>() * 8;
        let even = pixels(finest) * 16;
        let odd = levels.get(1).map_or(1, pixels) * 16;
        let patches = u64::from(finest.across) * u64::from(finest.down) * 16;
        let limits = device.limits();
        let most = limits
            .max_storage_buffer_binding_size
            .min(limits.max_buffer_size);
        if [luma, even, odd, patches].iter().any(|bytes| *bytes > most) {
            return None;
        }
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        Some(Work {
            luma: buffer("columbina flow luma", luma),
            even: buffer("columbina flow even", even),
            odd: buffer("columbina flow odd", odd),
            patches: buffer("columbina flow patches", patches),
            used: 0,
        })
    }
}

/// What finds the flows of a pair of pictures in a frame, before its layers are drawn
/// ([`Card::encode`]).
pub(crate) struct Job {
    /// What the flows kept are known by.
    id: u64,
    texel: Texel,
    levels: Vec<Level>,
    /// The uniforms of the levels, a slot each, which the binds read at their offsets.
    _params: wgpu::Buffer,
    /// What binds what the passes of the even levels read and write, and of the odd ones.
    binds: [wgpu::BindGroup; 2],
    /// What binds the planes of the two pictures.
    planes: wgpu::BindGroup,
}

/// The pipelines of the passes of flows, and the flows kept.
pub(crate) struct Card {
    layout: wgpu::BindGroupLayout,
    planes_layouts: HashMap<Texel, wgpu::BindGroupLayout>,
    bases: HashMap<Texel, wgpu::ComputePipeline>,
    halve: wgpu::ComputePipeline,
    patches: wgpu::ComputePipeline,
    dense: wgpu::ComputePipeline,
    store: wgpu::ComputePipeline,
    /// The bytes of a slot of the uniform of a level.
    slot: u32,
    works: HashMap<(u32, u32), Work>,
    kept: Vec<Kept>,
    /// What the next flows kept are known by.
    next: u64,
    /// How many times flows were found.
    computed: u64,
}

/// What binds the planes of two pictures read as `texel`s, for the first pass.
fn planes_layout(device: &wgpu::Device, texel: Texel) -> wgpu::BindGroupLayout {
    let plane = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: texel.sample_type(),
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("columbina flow pictures"),
        entries: &[plane(0), plane(1)],
    })
}

/// What binds what every pass reads and writes: the uniform of its level, the pyramid, the
/// flows of the coarser level, of the patches and of its level, and the texture of flows.
fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    };
    let storage = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: false },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: true,
        min_binding_size: wgpu::BufferSize::new(PARAMS_SIZE),
    };
    let flows = wgpu::BindingType::StorageTexture {
        access: wgpu::StorageTextureAccess::WriteOnly,
        format: FORMAT,
        view_dimension: wgpu::TextureViewDimension::D2,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("columbina flow"),
        entries: &[
            entry(0, uniform),
            entry(1, storage),
            entry(2, storage),
            entry(3, storage),
            entry(4, storage),
            entry(5, flows),
        ],
    })
}

/// The shader of the passes whose planes are read as `texel`s.
fn shader(texel: Texel) -> String {
    match texel {
        Texel::Float => SHADER.to_owned(),
        Texel::Uint => SHADER.replacen("alias Texel = f32;", "alias Texel = u32;", 1),
    }
}

/// The module of `source`, its errors of validation caught.
fn module(device: &wgpu::Device, source: String) -> Result<wgpu::ShaderModule, String> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("columbina flow"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    match pollster::block_on(scope.pop()) {
        Some(error) => Err(format!("the shader of flows: {error}")),
        None => Ok(module),
    }
}

/// The pipeline of the pass `entry` of `module`, binding `layouts`: made on the thread that
/// calls it, its errors of validation caught in a scope of that thread.
fn pipeline(
    device: &wgpu::Device,
    module: &wgpu::ShaderModule,
    layouts: &[Option<&wgpu::BindGroupLayout>],
    entry: &str,
) -> Result<wgpu::ComputePipeline, String> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("columbina flow"),
        bind_group_layouts: layouts,
        immediate_size: 0,
    });
    let made = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry),
        layout: Some(&layout),
        module,
        entry_point: Some(entry),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    match pollster::block_on(scope.pop()) {
        Some(error) => Err(format!("the pass {entry} of flows: {error}")),
        None => Ok(made),
    }
}

/// The groups of a pass over `count` invocations across or down.
fn groups(count: u32) -> u32 {
    count.div_ceil(GROUP).max(1)
}

/// What binds what a pass reads and writes: the uniforms of the levels `params`, a slot of which
/// is bound at a time, the pyramid and the flows of the patches of `work`, the flows of the
/// coarser level (the first of `levels`) and those of the level of the pass (the second), and
/// the texture of flows `view`.
fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    params: &wgpu::Buffer,
    work: &Work,
    levels: (&wgpu::Buffer, &wgpu::Buffer),
    view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let (coarse, dense) = levels;
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("columbina flow"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: params,
                    offset: 0,
                    size: wgpu::BufferSize::new(PARAMS_SIZE),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: work.luma.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: coarse.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: work.patches.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: dense.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(view),
            },
        ],
    })
}

impl Card {
    /// The pipelines of the passes of flows on `gpu`, made side by side, each on a thread of its
    /// own: FXC takes a while over each.
    ///
    /// # Errors
    /// The shader or a pipeline cannot be made, as where the card does not take it.
    pub(crate) fn new(gpu: &Gpu) -> Result<Card, String> {
        let device = gpu.device();
        let layout = layout(device);
        let planes_layouts: HashMap<Texel, wgpu::BindGroupLayout> = [Texel::Float, Texel::Uint]
            .into_iter()
            .map(|texel| (texel, planes_layout(device, texel)))
            .collect();
        let float = module(device, shader(Texel::Float))?;
        let uint = module(device, shader(Texel::Uint))?;
        let one = [Some(&layout)];
        let with_planes = |texel: Texel| [Some(&layout), Some(&planes_layouts[&texel])];
        let (float_planes, uint_planes) = (with_planes(Texel::Float), with_planes(Texel::Uint));
        let passes: [(&wgpu::ShaderModule, &[Option<&wgpu::BindGroupLayout>], &str); 6] = [
            (&float, &float_planes, "base_main"),
            (&uint, &uint_planes, "base_main"),
            (&float, &one, "halve_main"),
            (&float, &one, "patches_main"),
            (&float, &one, "dense_main"),
            (&float, &one, "store_main"),
        ];
        let made = std::thread::scope(|scope| {
            let making = passes.map(|(module, layouts, entry)| {
                scope.spawn(move || pipeline(device, module, layouts, entry))
            });
            making.map(|made| {
                made.join()
                    .unwrap_or_else(|_| Err("a pipeline of flows panicked".into()))
            })
        });
        let [of_floats, of_integers, halving, searching, densifying, storing] = made;
        let alignment = device.limits().min_uniform_buffer_offset_alignment;
        let slot = u32::try_from(PARAMS_SIZE)
            .unwrap_or(256)
            .next_multiple_of(alignment);
        Ok(Card {
            bases: HashMap::from([(Texel::Float, of_floats?), (Texel::Uint, of_integers?)]),
            halve: halving?,
            patches: searching?,
            dense: densifying?,
            store: storing?,
            layout,
            planes_layouts,
            slot,
            works: HashMap::new(),
            kept: Vec::new(),
            next: 0,
            computed: 0,
        })
    }

    /// How many times it has found flows since it was made: for the tests, that the frames
    /// between the same two pictures find them once.
    pub(crate) fn computed(&self) -> u64 {
        self.computed
    }

    /// The flows between `pictures`, which `pair` knows, for frame `frame`: those kept, or a
    /// texture for them and the job that finds them in this frame, before anything that reads
    /// them is drawn. Flows that were asked for in this frame already are found by the job of
    /// that frame, and those that a job was asked for in a frame before but that were not found,
    /// as where the frame was not drawn, are asked for again. None where the card cannot hold
    /// what the passes work in.
    pub(crate) fn flows(
        &mut self,
        gpu: &Gpu,
        pair: &Pair,
        pictures: &Pictures,
        frame: u64,
    ) -> Option<(Flows, Option<Job>)> {
        let size = pictures.size;
        let known = self
            .kept
            .iter()
            .position(|kept| kept.pair == *pair && kept.size == size);
        if let Some(at) = known {
            let kept = &mut self.kept[at];
            kept.used = frame;
            let flows = Flows {
                view: kept.view.clone(),
                scale: kept.scale,
            };
            if kept.found || kept.asked == frame {
                return Some((flows, None));
            }
            kept.asked = frame;
            let id = kept.id;
            let job = self.job(gpu, pictures, id, &flows.view, frame)?;
            return Some((flows, Some(job)));
        }
        let (scale, levels) = levels(size.0, size.1);
        let finest = (levels[0].width, levels[0].height);
        // Room for them: the flows read last longest ago, by no layer of this frame, are let
        // go of where the most are kept, and their texture is used again where it is of the
        // size.
        let mut texture = None;
        if self.kept.len() >= KEPT {
            let oldest = self
                .kept
                .iter()
                .enumerate()
                .filter(|(_, kept)| kept.used < frame)
                .min_by_key(|(_, kept)| kept.used)
                .map(|(at, _)| at);
            if let Some(at) = oldest {
                let old = self.kept.swap_remove(at);
                if (old.texture.width(), old.texture.height()) == finest {
                    texture = Some(old.texture);
                }
            }
        }
        let texture = texture.unwrap_or_else(|| flows_texture(gpu.device(), finest));
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let id = self.next;
        self.next += 1;
        let job = self.job(gpu, pictures, id, &view, frame)?;
        self.kept.push(Kept {
            pair: pair.clone(),
            size,
            texture,
            view: view.clone(),
            scale,
            used: frame,
            asked: frame,
            found: false,
            id,
        });
        Some((Flows { view, scale }, Some(job)))
    }

    /// The job that finds the flows between `pictures` into `view`, a texture of the size of
    /// the finest level of their pyramid, in frame `frame`; known by `id`. None where the card
    /// cannot hold what its passes work in.
    fn job(
        &mut self,
        gpu: &Gpu,
        pictures: &Pictures,
        id: u64,
        view: &wgpu::TextureView,
        frame: u64,
    ) -> Option<Job> {
        let device = gpu.device();
        let (scale, levels) = levels(pictures.size.0, pictures.size.1);
        let work = match self.works.entry((levels[0].width, levels[0].height)) {
            Entry::Occupied(work) => work.into_mut(),
            Entry::Vacant(room) => room.insert(Work::new(device, &levels)?),
        };
        work.used = frame;
        let slot = self.slot as usize;
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina flow levels"),
            size: (slot * levels.len()) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let written = uniforms(&levels, scale, pictures.size, pictures.decoded, slot);
        gpu.queue().write_buffer(&params, 0, &written);
        // An even level writes its flows into `even` and reads those of the odd level coarser
        // than it from `odd`; an odd one the other way round.
        let binds = [
            bind(
                device,
                &self.layout,
                &params,
                work,
                (&work.odd, &work.even),
                view,
            ),
            bind(
                device,
                &self.layout,
                &params,
                work,
                (&work.even, &work.odd),
                view,
            ),
        ];
        let [first, second] = [pictures.first, pictures.second]
            .map(|plane| plane.create_view(&wgpu::TextureViewDescriptor::default()));
        let planes = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina flow pictures"),
            layout: &self.planes_layouts[&pictures.texel],
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&first),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&second),
                },
            ],
        });
        Some(Job {
            id,
            texel: pictures.texel,
            levels,
            _params: params,
            binds,
            planes,
        })
    }

    /// Find the flows of `jobs` into `encoder`, one after another, before anything that reads
    /// them: the finest level of the pyramid of each, the coarser levels, the patches and the
    /// dense flows of each level from the coarsest to the finest, and the flows of the finest
    /// smoothed into their texture.
    pub(crate) fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, jobs: &[Job]) {
        if jobs.is_empty() {
            return;
        }
        let slot = self.slot;
        let offset = |level: usize| slot * u32::try_from(level).unwrap_or(0);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("columbina flow"),
                timestamp_writes: None,
            });
            for job in jobs {
                let finest = job.levels[0];
                pass.set_pipeline(&self.bases[&job.texel]);
                pass.set_bind_group(0, &job.binds[0], &[offset(0)]);
                pass.set_bind_group(1, &job.planes, &[]);
                pass.dispatch_workgroups(groups(finest.width), groups(finest.height), 1);
                pass.set_pipeline(&self.halve);
                for (index, level) in job.levels.iter().enumerate().skip(1) {
                    pass.set_bind_group(0, &job.binds[0], &[offset(index)]);
                    pass.dispatch_workgroups(groups(level.width), groups(level.height), 1);
                }
                for (index, level) in job.levels.iter().enumerate().rev() {
                    pass.set_bind_group(0, &job.binds[index % 2], &[offset(index)]);
                    pass.set_pipeline(&self.patches);
                    pass.dispatch_workgroups(groups(level.across), groups(level.down), 1);
                    pass.set_pipeline(&self.dense);
                    pass.dispatch_workgroups(groups(level.width), groups(level.height), 1);
                }
                pass.set_pipeline(&self.store);
                pass.set_bind_group(0, &job.binds[0], &[offset(0)]);
                pass.dispatch_workgroups(groups(finest.width), groups(finest.height), 1);
            }
        }
        self.computed += jobs.len() as u64;
        for kept in &mut self.kept {
            kept.found |= jobs.iter().any(|job| job.id == kept.id);
        }
    }

    /// Keep, after frame `frame`, the flows read last, as many as are kept, and none that no
    /// frame has read for a while; and what the passes work in, where flows of its size were
    /// found a while ago at most.
    pub(crate) fn forget(&mut self, frame: u64) {
        self.kept.sort_by_key(|kept| std::cmp::Reverse(kept.used));
        self.kept.truncate(KEPT);
        self.kept.retain(|kept| kept.used + KEPT_FOR >= frame);
        self.works.retain(|_, work| work.used + KEPT_FOR >= frame);
    }
}

/// A texture of flows of `size`, which the passes write and the layer reads.
fn flows_texture(device: &wgpu::Device, size: (u32, u32)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("columbina flows"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// The uniforms of `levels` of the pyramid of pictures of `size`, a pixel of the finest `scale`
/// pixels of them across and down, whose codes are read as `decoded` says (the first part of
/// their uniform of `yuv.wgsl`): a slot of `slot` bytes each, as `Params` of `flow.wgsl` lays
/// them out.
fn uniforms(
    levels: &[Level],
    scale: u16,
    size: (u32, u32),
    decoded: &[f32],
    slot: usize,
) -> Vec<u8> {
    let kind = if decoded[KIND_PARAM] > 1.5 {
        2
    } else {
        u32::from(decoded[KIND_PARAM] > 0.5)
    };
    let [red, green, blue] = crate::grade::REC709_LUMA;
    let mut bytes = vec![0_u8; slot * levels.len()];
    for (index, (level, room)) in levels.iter().zip(bytes.chunks_mut(slot)).enumerate() {
        let coarse = levels.get(index + 1);
        let finer = index.checked_sub(1).and_then(|at| levels.get(at));
        let words: [u32; 20] = [
            level.width,
            level.height,
            level.luma,
            0,
            level.across,
            level.down,
            0,
            0,
            coarse.map_or(0, |coarse| coarse.width),
            coarse.map_or(0, |coarse| coarse.height),
            u32::from(coarse.is_some()),
            0,
            finer.map_or(0, |finer| finer.width),
            finer.map_or(0, |finer| finer.height),
            finer.map_or(0, |finer| finer.luma),
            0,
            size.0,
            size.1,
            u32::from(scale),
            kind,
        ];
        let floats: [f32; 8] = [
            decoded[SCALE_PARAM],
            decoded[BLACK_PARAM],
            decoded[RANGE_PARAM],
            decoded[UNIT_PARAM],
            f32::from(ITERATIONS),
            red,
            green,
            blue,
        ];
        let values = words
            .iter()
            .map(|word| word.to_le_bytes())
            .chain(floats.iter().map(|float| float.to_le_bytes()));
        for (value, at) in values.zip(room.chunks_mut(4)) {
            at.copy_from_slice(&value);
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_finest_level_is_the_pictures_up_to_2k_and_the_coarsest_is_from_32_to_63_pixels_long() {
        let sizes = |width, height| -> (u16, Vec<(u32, u32)>) {
            let (scale, levels) = levels(width, height);
            let each = levels
                .iter()
                .map(|level| (level.width, level.height))
                .collect();
            (scale, each)
        };
        let hd = vec![
            (1920, 1080),
            (960, 540),
            (480, 270),
            (240, 135),
            (120, 68),
            (60, 34),
        ];
        assert_eq!(sizes(1920, 1080), (1, hd.clone()));
        // 4K is halved first, and has the levels of HD.
        assert_eq!(sizes(3840, 2160), (2, hd));
        assert_eq!(sizes(4096, 2160).0, 2);
        assert_eq!(sizes(7680, 4320).0, 4);
        assert_eq!(
            sizes(256, 192),
            (1, vec![(256, 192), (128, 96), (64, 48), (32, 24)])
        );
        // A picture smaller than the coarsest level is a level of its own.
        assert_eq!(sizes(16, 8), (1, vec![(16, 8)]));
        assert_eq!(sizes(1, 1), (1, vec![(1, 1)]));
        // Upright, as a picture turned a quarter is stored.
        assert_eq!(sizes(1080, 1920).1[5], (34, 60));
        // Never more than the most levels, and the luma of each after that of the one before.
        let (_, tall) = levels(2048, 16);
        assert!(tall.len() <= MOST_LEVELS);
        for pair in tall.windows(2) {
            assert_eq!(pair[1].luma, pair[0].luma + pair[0].width * pair[0].height);
        }
    }

    #[test]
    fn patches_begin_every_4_pixels_and_the_last_lies_against_the_far_edge() {
        // 30 pixels: patches at 0, 4, … 20 and the last at 22, which ends at the edge.
        assert_eq!(patches(30), 7);
        assert_eq!(patches(32), 7);
        assert_eq!(patches(33), 8);
        // A row of a patch or less has one.
        assert_eq!(patches(8), 1);
        assert_eq!(patches(3), 1);
        assert_eq!(patches(12), 2);
    }

    #[test]
    fn the_uniform_of_a_level_says_its_size_its_neighbours_and_how_the_pictures_are_read() {
        let (scale, levels) = levels(256, 192);
        // Of R'G'B'A of floats, as `yuv::picture_params` lays it out.
        let decoded = crate::yuv::picture_params(
            [0.0; 4],
            crate::yuv::Layout {
                width: 256,
                height: 192,
                format: columbina_media::PictureFormat::RgbaF16,
                bits: 16,
                chroma_location: 0,
                alpha: columbina_media::Alpha::Opaque,
            },
            Texel::Float,
            crate::YuvCoding::default(),
            0,
        );
        let bytes = uniforms(&levels, scale, (256, 192), &decoded, 256);
        assert_eq!(bytes.len(), 256 * 4);
        let word = |slot: usize, at: usize| {
            let start = slot * 256 + at * 4;
            u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap())
        };
        let float = |slot: usize, at: usize| f32::from_bits(word(slot, at));
        // The second level: 128 × 96, after the 256 × 192 pairs of the first; 31 × 23 patches.
        assert_eq!([word(1, 0), word(1, 1), word(1, 2)], [128, 96, 256 * 192]);
        assert_eq!([word(1, 4), word(1, 5)], [31, 23]);
        // Coarser than it, the third; finer, the first.
        assert_eq!([word(1, 8), word(1, 9), word(1, 10)], [64, 48, 1]);
        assert_eq!([word(1, 12), word(1, 13), word(1, 14)], [256, 192, 0]);
        // The coarsest has none coarser.
        assert_eq!(word(3, 10), 0);
        // The pictures, a pixel of the finest level a pixel of theirs, stored as floats.
        let picture = [word(2, 16), word(2, 17), word(2, 18), word(2, 19)];
        assert_eq!(picture, [256, 192, 1, 2]);
        // Read as `yuv.wgsl` reads them: floats from 0 to 1, as they are.
        let codes = [float(0, 20), float(0, 21), float(0, 22), float(0, 23)];
        assert_eq!(codes[1..], [0.0, 1.0, 1.0]);
        assert_eq!(float(0, 24), f32::from(ITERATIONS));
        assert_eq!(
            [float(0, 25), float(0, 26), float(0, 27)],
            crate::grade::REC709_LUMA
        );
    }

    /// `code` parsed and validated by naga, as wgpu would.
    fn validated(code: &str) -> Result<(), String> {
        let module =
            naga::front::wgsl::parse_str(code).map_err(|error| error.emit_to_string(code))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|error| format!("{:?}", error.into_inner()))?;
        Ok(())
    }

    #[test]
    fn the_shader_of_the_passes_is_wgsl_that_the_card_takes_of_planes_of_either_texel() {
        for texel in [Texel::Float, Texel::Uint] {
            let code = shader(texel);
            validated(&code).unwrap_or_else(|why| panic!("{texel:?}: {why}"));
            let alias = if texel == Texel::Uint { "u32" } else { "f32" };
            assert!(code.contains(&format!("\nalias Texel = {alias};")));
            for entry in [
                "base_main",
                "halve_main",
                "patches_main",
                "dense_main",
                "store_main",
            ] {
                assert!(code.contains(&format!("fn {entry}(")), "{entry}");
            }
        }
        // The sizes that it is laid out by are those of the shader.
        assert!(SHADER.contains(&format!("const PATCH: i32 = {PATCH};")));
        assert!(SHADER.contains(&format!("const STRIDE: i32 = {STRIDE};")));
        let group = format!("@workgroup_size({GROUP}, {GROUP}, 1)");
        assert_eq!(SHADER.matches(&group).count(), 5);
        let fields = SHADER.split("struct Params {").nth(1).unwrap();
        let fields = fields.split("};").next().unwrap();
        assert_eq!(fields.matches(": vec4<").count() as u64 * 16, PARAMS_SIZE);
    }
}
