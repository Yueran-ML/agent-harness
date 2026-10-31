//! The scopes on the card: the pipelines, made once ([`Scopes`]), and the buffers and picture of
//! each scope shown ([`Scope`]).

use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use super::reference::{self, Coefficients, Vector};
use super::{
    graticule, Bins, Channel, Chroma, Graticule, Kind, Mapping, Sampling, ScopeError,
    ScopeSettings, Signal, Statistics, WaveformChannels,
};
use crate::Gpu;

const SHADER: &str = include_str!("scopes.wgsl");

/// The words of the parameters (`Params` of `scopes.wgsl`).
const PARAMS: usize = 39;
/// As in `scopes.wgsl`: pixels down that a thread counts.
const RUN: u32 = 8;
const HISTOGRAM_RUN: u32 = 32;
const STATISTICS_RUN: u32 = 16;
/// The largest frame a scope reads, each way.
const LARGEST_FRAME: u32 = 16384;
/// The format of a scope's picture.
const PICTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How long the card is given to map a buffer for reading before it is taken as hung.
const READ_WITHIN: Duration = Duration::from_secs(20);
/// How long to wait between asking the card whether a buffer is mapped.
const READ_POLL: Duration = Duration::from_micros(250);

/// The pipelines of the scopes, made once for the card (some 10 ms) and shared by every scope.
pub struct Scopes {
    gpu: Arc<Gpu>,
    accumulate_layout: wgpu::BindGroupLayout,
    display_layout: wgpu::BindGroupLayout,
    statistics_layout: wgpu::BindGroupLayout,
    waveform: wgpu::ComputePipeline,
    vectorscope: wgpu::ComputePipeline,
    histogram: wgpu::ComputePipeline,
    chromaticity: wgpu::ComputePipeline,
    display: wgpu::ComputePipeline,
    statistics: wgpu::ComputePipeline,
    statistics_final: wgpu::ComputePipeline,
    /// Bound where a scope has no table of linear light.
    no_table: wgpu::Buffer,
}

impl std::fmt::Debug for Scopes {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Scopes").finish_non_exhaustive()
    }
}

/// A scope shown: its settings, its counts and its picture on the card. Made by
/// [`Scopes::scope`]; other settings make another.
pub struct Scope {
    settings: ScopeSettings,
    graticule: Graticule,
    coefficients: Coefficients,
    matrix: [i32; 9],
    counts: wgpu::Buffer,
    params: wgpu::Buffer,
    table: Option<wgpu::Buffer>,
    overlay: wgpu::TextureView,
    picture: Arc<wgpu::Texture>,
    picture_view: wgpu::TextureView,
}

impl std::fmt::Debug for Scope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Scope")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// A picture of a scope read back: RGBA of 8 bits by rows from the top, opaque.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Scope {
    #[must_use]
    pub fn settings(&self) -> &ScopeSettings {
        &self.settings
    }

    /// The graticule, in pixels of the picture, for an interface to draw over it.
    #[must_use]
    pub fn graticule(&self) -> &Graticule {
        &self.graticule
    }

    /// The picture of the scope on the card (RGBA of 8 bits, which views can draw from), as the
    /// last [`Scopes::encode`] left it once its commands are done.
    #[must_use]
    pub fn picture(&self) -> &Arc<wgpu::Texture> {
        &self.picture
    }

    /// The size of the picture.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.settings.kind.picture_size()
    }
}

/// The parameters of a pass, as `Params` of `scopes.wgsl` has them.
#[derive(Clone, Copy, Default)]
struct Params {
    frame: [u32; 2],
    step: u32,
    sampled: [u32; 2],
    grid: [u32; 2],
    planes: u32,
    coefficients: [i32; 5],
    zoom: i32,
    tones: u32,
    tone: [i32; 2],
    matrix: [i32; 9],
    display: u32,
    mapping: u32,
    gain: f32,
    graticule: f32,
    samples: f32,
    image: [u32; 2],
    luma: [f32; 3],
    partials: u32,
    strip: u32,
    channels: u32,
}

impl Params {
    fn bytes(&self) -> Vec<u8> {
        let signed = |value: i32| u32::from_ne_bytes(value.to_ne_bytes());
        let mut words = Vec::with_capacity(PARAMS);
        words.extend(self.frame);
        words.push(self.step);
        words.extend(self.sampled);
        words.extend(self.grid);
        words.push(self.planes);
        words.extend(self.coefficients.map(signed));
        words.push(signed(self.zoom));
        words.push(self.tones);
        words.extend(self.tone.map(signed));
        words.extend(self.matrix.map(signed));
        words.push(self.display);
        words.push(self.mapping);
        words.push(self.gain.to_bits());
        words.push(self.graticule.to_bits());
        words.push(self.samples.to_bits());
        words.extend(self.image);
        words.extend(self.luma.map(f32::to_bits));
        words.push(self.partials);
        words.push(self.strip);
        words.push(self.channels);
        debug_assert_eq!(words.len(), PARAMS);
        words.into_iter().flat_map(u32::to_le_bytes).collect()
    }
}

impl Scopes {
    /// Make the pipelines of the scopes on `gpu`.
    #[must_use]
    pub fn new(gpu: Arc<Gpu>) -> Scopes {
        let device = gpu.device();
        let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty,
            count: None,
        };
        let storage = |read_only: bool| wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        };
        let texture = wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        };
        let layout = |label: &str, entries: &[wgpu::BindGroupLayoutEntry]| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries,
            })
        };
        let accumulate_layout = layout(
            "columbina scopes count",
            &[
                entry(0, texture),
                entry(1, storage(true)),
                entry(2, storage(false)),
                entry(3, storage(true)),
            ],
        );
        let display_layout = layout(
            "columbina scopes draw",
            &[
                entry(1, storage(true)),
                entry(2, storage(false)),
                entry(
                    4,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: PICTURE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                entry(5, texture),
            ],
        );
        let statistics_layout = layout(
            "columbina scopes statistics",
            &[
                entry(0, texture),
                entry(1, storage(true)),
                entry(6, storage(false)),
                entry(7, storage(false)),
            ],
        );
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("columbina scopes"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry_point: &str| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("columbina scopes"),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let no_table = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina scopes no table"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        Scopes {
            waveform: pipeline(&accumulate_layout, "waveform_main"),
            vectorscope: pipeline(&accumulate_layout, "vectorscope_main"),
            histogram: pipeline(&accumulate_layout, "histogram_main"),
            chromaticity: pipeline(&accumulate_layout, "chromaticity_main"),
            display: pipeline(&display_layout, "display_main"),
            statistics: pipeline(&statistics_layout, "statistics_main"),
            statistics_final: pipeline(&statistics_layout, "statistics_final"),
            accumulate_layout,
            display_layout,
            statistics_layout,
            no_table,
            gpu,
        }
    }

    #[must_use]
    pub fn gpu(&self) -> &Arc<Gpu> {
        &self.gpu
    }

    /// Make a scope: its buffers, its picture, and its graticule drawn.
    ///
    /// # Errors
    /// The settings are out of range ([`ScopeSettings::check`]).
    pub fn scope(&self, settings: ScopeSettings) -> Result<Scope, ScopeError> {
        settings.check().map_err(ScopeError::Settings)?;
        let device = self.gpu.device();
        let queue = self.gpu.queue();
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina scope counts"),
            size: (settings.kind.count_size() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina scope parameters"),
            size: (PARAMS * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (table, matrix) = if let Kind::Chromaticity(diagram) = &settings.kind {
            let integers = reference::Chromaticity::of(
                diagram.width,
                diagram.height,
                &diagram.space,
                settings.signal.transfer,
            );
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("columbina scope linear light"),
                size: (integers.table.len() * 4) as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bytes: Vec<u8> = integers
                .table
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            queue.write_buffer(&buffer, 0, &bytes);
            (Some(buffer), integers.matrix)
        } else {
            (None, [0; 9])
        };
        let (width, height) = settings.kind.picture_size();
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let graticule = graticule::graticule(&settings);
        let overlay = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("columbina scope graticule"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICTURE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            overlay.as_image_copy(),
            &graticule::rasterize(&graticule),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            size,
        );
        let picture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("columbina scope picture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICTURE_FORMAT,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Scope {
            coefficients: Coefficients::of(settings.signal.luma),
            matrix,
            graticule,
            counts,
            params,
            table,
            overlay: overlay.create_view(&wgpu::TextureViewDescriptor::default()),
            picture_view: picture.create_view(&wgpu::TextureViewDescriptor::default()),
            picture: Arc::new(picture),
            settings,
        })
    }

    /// Put the commands that count `frame` into `scope` and draw its picture into `encoder`: a
    /// scope once an encoder (its parameters are written to the queue, so for the submission
    /// that comes next).
    ///
    /// # Errors
    /// The frame is not a texture of two dimensions, of floats, that can be bound, from 1 to
    /// 16384 pixels each way.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        scope: &Scope,
        sampling: Sampling,
    ) -> Result<(), ScopeError> {
        let (frame_width, frame_height) = frame_size(frame)?;
        let step = sampling.step();
        let sampled = [frame_width.div_ceil(step), frame_height.div_ceil(step)];
        let params = scope.params([frame_width, frame_height], step, sampled);
        self.gpu
            .queue()
            .write_buffer(&scope.params, 0, &params.bytes());
        encoder.clear_buffer(&scope.counts, 0, None);
        let device = self.gpu.device();
        let frame_view = frame.create_view(&wgpu::TextureViewDescriptor::default());
        let accumulate = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina scope count"),
            layout: &self.accumulate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&frame_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scope.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scope.counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: scope
                        .table
                        .as_ref()
                        .unwrap_or(&self.no_table)
                        .as_entire_binding(),
                },
            ],
        });
        let display = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina scope draw"),
            layout: &self.display_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scope.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: scope.counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&scope.picture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&scope.overlay),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("columbina scope"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &accumulate, &[]);
        let (pipeline, groups) = match &scope.settings.kind {
            Kind::Waveform(_) | Kind::Parade(_) => (&self.waveform, [params.grid[0], 1]),
            Kind::Vectorscope(_) => (
                &self.vectorscope,
                [sampled[0].div_ceil(16), sampled[1].div_ceil(16 * RUN)],
            ),
            Kind::Histogram(_) => (
                &self.histogram,
                [
                    sampled[0].div_ceil(16),
                    sampled[1].div_ceil(16 * HISTOGRAM_RUN),
                ],
            ),
            Kind::Chromaticity(_) => (
                &self.chromaticity,
                [sampled[0].div_ceil(16), sampled[1].div_ceil(16 * RUN)],
            ),
        };
        pass.set_pipeline(pipeline);
        pass.dispatch_workgroups(groups[0], groups[1], 1);
        pass.set_bind_group(0, &display, &[]);
        pass.set_pipeline(&self.display);
        pass.dispatch_workgroups(
            params.image[0].div_ceil(16),
            params.image[1].div_ceil(16),
            1,
        );
        Ok(())
    }

    /// Count `frame` into `scope` and draw its picture, now.
    ///
    /// # Errors
    /// As [`Scopes::encode`].
    pub fn draw(
        &self,
        frame: &wgpu::Texture,
        scope: &Scope,
        sampling: Sampling,
    ) -> Result<(), ScopeError> {
        let mut encoder =
            self.gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("columbina scope"),
                });
        self.encode(&mut encoder, frame, scope, sampling)?;
        self.gpu.queue().submit([encoder.finish()]);
        Ok(())
    }

    /// Read back the picture of a scope, once what was drawn into it is done.
    ///
    /// # Errors
    /// The device was lost, or the card did not give the picture back.
    pub fn read_image(&self, scope: &Scope) -> Result<ScopeImage, ScopeError> {
        let (width, height) = scope.size();
        let row = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let read = self.read_buffer(u64::from(row) * u64::from(height));
        let mut encoder =
            self.gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("columbina scope read"),
                });
        encoder.copy_texture_to_buffer(
            scope.picture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            scope.picture.size(),
        );
        self.gpu.queue().submit([encoder.finish()]);
        let bytes = self.mapped(&read)?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for line in bytes.chunks(row as usize) {
            rgba.extend_from_slice(&line[..(width * 4) as usize]);
        }
        Ok(ScopeImage {
            width,
            height,
            rgba,
        })
    }

    /// Read back the counts of a scope, once what was counted is done. They are laid out:
    /// - a waveform: by plane (luma alone, or R, G and B), then by level (from a signal of 0 up),
    ///   then by column; a parade likewise, R, G and B, the columns being those of each;
    /// - a vectorscope: by row from the top (Cr down from 0.5), then by column (Cb from −0.5);
    /// - a histogram: R, G, B and luma, by bin each, then the highest count of each;
    /// - a chromaticity diagram: by row from the top (y down from 0.9), then by column (x from 0).
    ///
    /// # Errors
    /// The device was lost, or the card did not give the counts back.
    pub fn read_counts(&self, scope: &Scope) -> Result<Vec<u32>, ScopeError> {
        let size = scope.counts.size();
        let read = self.read_buffer(size);
        let mut encoder =
            self.gpu
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("columbina scope read"),
                });
        encoder.copy_buffer_to_buffer(&scope.counts, 0, &read, 0, size);
        self.gpu.queue().submit([encoder.finish()]);
        Ok(words(&self.mapped(&read)?))
    }

    /// The figures of every pixel of `frame` (see [`Statistics`]), its luma and chroma made by
    /// `signal`, with a histogram of `bins`; worked out on the card, and read back now.
    ///
    /// # Errors
    /// The signal or the bins are out of range, the frame is not one a scope reads (see
    /// [`Scopes::encode`]), the device was lost, or the card did not give the figures back.
    #[allow(clippy::too_many_lines, reason = "three passes and what they read")]
    pub fn statistics(
        &self,
        frame: &wgpu::Texture,
        signal: &Signal,
        bins: u32,
    ) -> Result<Statistics, ScopeError> {
        super::check_signal(signal).map_err(ScopeError::Settings)?;
        if !(16..=1024).contains(&bins) {
            return Err(ScopeError::Settings(format!(
                "the bins of a histogram are {bins}, not from 16 to 1024"
            )));
        }
        let (width, height) = frame_size(frame)?;
        let groups = [width.div_ceil(16), height.div_ceil(16 * STATISTICS_RUN)];
        let partials = groups[0] * groups[1];
        let coefficients = Coefficients::of(signal.luma);
        let params = Params {
            frame: [width, height],
            step: 1,
            sampled: [width, height],
            grid: [bins, 1],
            coefficients: [
                coefficients.kr,
                coefficients.kg,
                coefficients.kb,
                coefficients.cb,
                coefficients.cr,
            ],
            luma: signal.luma,
            partials,
            ..Params::default()
        };
        let device = self.gpu.device();
        let buffer = |label: &str, size: u64, usage: wgpu::BufferUsages| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let storage = wgpu::BufferUsages::STORAGE;
        let params_buffer = buffer(
            "columbina statistics parameters",
            (PARAMS * 4) as u64,
            storage | wgpu::BufferUsages::COPY_DST,
        );
        self.gpu
            .queue()
            .write_buffer(&params_buffer, 0, &params.bytes());
        let counts_size = u64::from(4 * bins + 4) * 4;
        let counts = buffer(
            "columbina statistics histogram",
            counts_size,
            storage | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
        );
        let partial_values = buffer(
            "columbina statistics partials",
            u64::from(partials) * 5 * 16,
            storage,
        );
        let results = buffer(
            "columbina statistics",
            5 * 16,
            storage | wgpu::BufferUsages::COPY_SRC,
        );
        let counts_read = self.read_buffer(counts_size);
        let results_read = self.read_buffer(5 * 16);
        let frame_view = frame.create_view(&wgpu::TextureViewDescriptor::default());
        let accumulate = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina statistics histogram"),
            layout: &self.accumulate_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&frame_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: counts.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.no_table.as_entire_binding(),
                },
            ],
        });
        let sums = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("columbina statistics"),
            layout: &self.statistics_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&frame_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: partial_values.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: results.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("columbina statistics"),
        });
        encoder.clear_buffer(&counts, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("columbina statistics"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, &accumulate, &[]);
            pass.set_pipeline(&self.histogram);
            pass.dispatch_workgroups(width.div_ceil(16), height.div_ceil(16 * HISTOGRAM_RUN), 1);
            pass.set_bind_group(0, &sums, &[]);
            pass.set_pipeline(&self.statistics);
            pass.dispatch_workgroups(groups[0], groups[1], 1);
            pass.set_pipeline(&self.statistics_final);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&counts, 0, &counts_read, 0, counts_size);
        encoder.copy_buffer_to_buffer(&results, 0, &results_read, 0, 5 * 16);
        self.gpu.queue().submit([encoder.finish()]);
        let histogram = words(&self.mapped(&counts_read)?);
        let values: Vec<f32> = words(&self.mapped(&results_read)?)
            .into_iter()
            .map(f32::from_bits)
            .collect();
        let samples = u64::from(width) * u64::from(height);
        #[allow(clippy::cast_precision_loss, reason = "a count of pixels")]
        let count = samples as f64;
        let channel = |index: usize| Channel {
            min: values[4 + index],
            max: values[8 + index],
            mean: f64::from(values[index]) / count,
        };
        let mean = |index: usize| f64::from(values[12 + index]) / count;
        Ok(Statistics {
            samples,
            red: channel(0),
            green: channel(1),
            blue: channel(2),
            luma: channel(3),
            chroma: Chroma::of(
                [mean(0), mean(1)],
                [mean(2), mean(3)],
                f64::from(values[16]) / count,
            ),
            histogram: Bins::of(bins, &histogram),
        })
    }

    fn read_buffer(&self, size: u64) -> wgpu::Buffer {
        self.gpu.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("columbina scope read"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Map `buffer` for reading, wait until it is, and take its bytes: asking the card again
    /// and again, a moment apart, without blocking in it (a poll that waits holds a lock of the
    /// device that the views take to present).
    fn mapped(&self, buffer: &wgpu::Buffer) -> Result<Vec<u8>, ScopeError> {
        if let Some(why) = self.gpu.lost_reason() {
            return Err(ScopeError::Lost(why));
        }
        let (mapped, mapping) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = mapped.send(result);
            });
        let deadline = Instant::now() + READ_WITHIN;
        loop {
            self.gpu
                .device()
                .poll(wgpu::PollType::Poll)
                .map_err(|error| ScopeError::Read(error.to_string()))?;
            match mapping.try_recv() {
                Ok(Ok(())) => break,
                Ok(Err(error)) => return Err(ScopeError::Read(error.to_string())),
                Err(mpsc::TryRecvError::Disconnected) => {
                    return Err(ScopeError::Read("the mapping was dropped".into()));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if let Some(why) = self.gpu.lost_reason() {
                return Err(ScopeError::Lost(why));
            }
            if Instant::now() > deadline {
                return Err(ScopeError::Read("the card did not map the buffer".into()));
            }
            std::thread::sleep(READ_POLL);
        }
        let bytes = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|error| ScopeError::Read(error.to_string()))?
            .to_vec();
        buffer.unmap();
        Ok(bytes)
    }
}

impl Scope {
    /// The parameters of counting a frame of `frame` pixels, one in `step` (`sampled` pixels),
    /// and drawing the picture.
    #[allow(clippy::cast_precision_loss, reason = "a count of pixels")]
    fn params(&self, frame: [u32; 2], step: u32, sampled: [u32; 2]) -> Params {
        let settings = &self.settings;
        let c = self.coefficients;
        let (image_width, image_height) = settings.kind.picture_size();
        let look = &settings.look;
        let mut params = Params {
            frame,
            step,
            sampled,
            coefficients: [c.kr, c.kg, c.kb, c.cb, c.cr],
            matrix: self.matrix,
            mapping: match look.mapping {
                Mapping::Log => 0,
                Mapping::Gamma => 1,
            },
            gain: (16.0 * (look.brightness - 0.5)).exp2(),
            graticule: look.graticule,
            samples: (u64::from(sampled[0]) * u64::from(sampled[1])) as f32,
            image: [image_width, image_height],
            luma: settings.signal.luma,
            ..Params::default()
        };
        match &settings.kind {
            Kind::Waveform(waveform) => {
                params.grid = [waveform.width, waveform.levels];
                (params.planes, params.display) = match waveform.channels {
                    WaveformChannels::Luma => (1, 0),
                    WaveformChannels::Rgb => (3, 1),
                };
            }
            Kind::Parade(parade) => {
                params.grid = [parade.width, parade.levels];
                params.planes = 3;
                params.display = 2;
            }
            Kind::Vectorscope(scope) => {
                let vector = Vector::of(scope.size, scope.zoom, scope.tones, scope.low, scope.high);
                params.grid = [scope.size, scope.size];
                params.zoom = vector.zoom;
                params.tones = match scope.tones {
                    super::Tones::All => 0,
                    super::Tones::Low => 1,
                    super::Tones::Mid => 2,
                    super::Tones::High => 3,
                };
                params.tone = [vector.low, vector.high];
                params.display = 3;
            }
            Kind::Histogram(histogram) => {
                let strips = histogram.channels.count();
                params.grid = [histogram.bins, 1];
                params.display = 4;
                params.strip = histogram.height / strips;
                params.channels = strips;
            }
            Kind::Chromaticity(diagram) => {
                params.grid = [diagram.width, diagram.height];
                params.display = 5;
            }
        }
        params
    }
}

/// The size of a frame a scope reads.
fn frame_size(frame: &wgpu::Texture) -> Result<(u32, u32), ScopeError> {
    let refuse = |why: &str| Err(ScopeError::Frame(why.into()));
    if frame.dimension() != wgpu::TextureDimension::D2 || frame.depth_or_array_layers() != 1 {
        return refuse("a frame is a texture of two dimensions");
    }
    if frame.sample_count() != 1 || !frame.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) {
        return refuse("a frame is a texture of one sample that can be bound");
    }
    if !matches!(
        frame.format().sample_type(None, None),
        Some(wgpu::TextureSampleType::Float { .. })
    ) {
        return refuse("a frame is a texture of floats");
    }
    let (width, height) = (frame.width(), frame.height());
    if !(1..=LARGEST_FRAME).contains(&width) || !(1..=LARGEST_FRAME).contains(&height) {
        return refuse("a frame is from 1 to 16384 pixels each way");
    }
    Ok((width, height))
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect()
}
