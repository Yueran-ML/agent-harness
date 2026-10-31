//! The translated shaders on a card, against OCIO on the processor: every processor of the
//! catalogue to and from ACES2065-1 and every ACES 2.0 view, each made into the program the
//! compositor makes of an operation (`common::program`: its library, group 1 of parameters, a
//! linear sampler and filterable textures, `colour()`), run by a compute shader on colours of
//! its input's range.
//!
//! Held to 1e-3 where the output is of SDR (an encoding of 0 to 1), and to 1e-3 of the colour's
//! largest channel where it is linear light, log or HDR (absolutely where that is below 1), for
//! all but one channel in a thousand; the rest to 1e-2 (see [`OUTLIERS`]). The tables are half
//! floats as the compositor binds them, each split in two (see `columbina_ocio::gpu`); with
//! `Rgba32Float`, where the card can filter it, whole.
//!
//! With no card there is nothing to run, and the test says so. Run it alone on the card (the
//! GPU lock of the plan): `--test-threads=1`.

mod common;

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use columbina_lut::gpu::{Dimension, TexelFormat, TextureData};
use columbina_ocio::gpu::{Bake, GpuOp, Shaper};
use columbina_ocio::{Config, Direction};

struct Card {
    device: wgpu::Device,
    queue: wgpu::Queue,
    errors: Arc<Mutex<Vec<String>>>,
    float32_filterable: bool,
}

fn card() -> Option<Card> {
    if !wgpu::Instance::enabled_backend_features().intersects(wgpu::Backends::PRIMARY) {
        return None;
    }
    let mut descriptor = wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    };
    // As the engine's instance (columbina_compose::gpu::dx12_compiler, which this crate cannot
    // depend on): FXC, which Windows has; COLUMBINA_DX12_COMPILER=auto for wgpu's Auto, which
    // takes any dxcompiler.dll on PATH (one that an Electron app put there, a "Google Dawn
    // Custom Build" of DXC 1.8.2505, crashes on the ACES 2.0 shaders); or the path of a
    // dxcompiler.dll.
    descriptor.backend_options.dx12.shader_compiler = match std::env::var("COLUMBINA_DX12_COMPILER")
    {
        Ok(value) if value.eq_ignore_ascii_case("auto") => wgpu::Dx12Compiler::Auto,
        Ok(path) if path.to_ascii_lowercase().ends_with("dxcompiler.dll") => {
            wgpu::Dx12Compiler::DynamicDxc { dxc_path: path }
        }
        _ => wgpu::Dx12Compiler::Fxc,
    };
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .ok()?;
    eprintln!(
        "on {} ({:?})",
        adapter.get_info().name,
        adapter.get_info().backend
    );
    let float32_filterable = adapter
        .features()
        .contains(wgpu::Features::FLOAT32_FILTERABLE);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("columbina-ocio test"),
        required_features: if float32_filterable {
            wgpu::Features::FLOAT32_FILTERABLE
        } else {
            wgpu::Features::empty()
        },
        ..wgpu::DeviceDescriptor::default()
    }))
    .ok()?;
    let errors = Arc::new(Mutex::new(Vec::new()));
    device.on_uncaptured_error(Arc::new({
        let errors = Arc::clone(&errors);
        move |error: wgpu::Error| {
            eprintln!("wgpu: {error}");
            errors.lock().unwrap().push(error.to_string());
        }
    }));
    Some(Card {
        device,
        queue,
        errors,
        float32_filterable,
    })
}

fn require_card() -> Option<Card> {
    let found = card();
    if found.is_none() {
        assert!(
            std::env::var_os("COLUMBINA_REQUIRE_GPU").is_none(),
            "no card, and COLUMBINA_REQUIRE_GPU is set"
        );
        eprintln!("no card: nothing to run");
    }
    found
}

/// The compositor's program of `op`, with a compute shader that runs it on each input.
fn shader(op: &GpuOp) -> String {
    let mut code = common::program(&[op]);
    code.push_str(
        "@group(0) @binding(0) var<storage, read> inputs: array<vec4<f32>>;\n\
         @group(0) @binding(1) var<storage, read_write> outputs: array<vec4<f32>>;\n\
         @compute @workgroup_size(64)\n\
         fn main(@builtin(global_invocation_id) id: vec3<u32>) {\n    \
             if id.x >= arrayLength(&inputs) { return; }\n    \
             outputs[id.x] = vec4<f32>(colour(inputs[id.x].xyz), 1.0);\n\
         }\n",
    );
    code
}

fn texture(card: &Card, data: &TextureData) -> wgpu::TextureView {
    let texture = card.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ocio table"),
        size: wgpu::Extent3d {
            width: data.width,
            height: data.height,
            depth_or_array_layers: data.depth,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: match data.dimension {
            Dimension::D2 => wgpu::TextureDimension::D2,
            Dimension::D3 => wgpu::TextureDimension::D3,
        },
        format: match data.format {
            TexelFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
            TexelFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    card.queue.write_texture(
        texture.as_image_copy(),
        &data.bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(data.bytes_per_row()),
            rows_per_image: Some(data.height),
        },
        texture.size(),
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn storage(card: &Card, values: &[[f32; 4]], usage: wgpu::BufferUsages) -> wgpu::Buffer {
    let bytes: Vec<u8> = values
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let buffer = card.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ocio test"),
        size: bytes.len().max(16) as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    card.queue.write_buffer(&buffer, 0, &bytes);
    buffer
}

/// A program of an operation, ready to run.
struct Ready {
    pipeline: wgpu::ComputePipeline,
    io_layout: wgpu::BindGroupLayout,
    colour: wgpu::BindGroup,
}

#[allow(clippy::too_many_lines)]
fn prepare(card: &Card, op: &GpuOp) -> Ready {
    let device = &card.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ocio test"),
        source: wgpu::ShaderSource::Wgsl(shader(op).into()),
    });
    let buffer_entry = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let io_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ocio test io"),
        entries: &[buffer_entry(0, true), buffer_entry(1, false)],
    });
    // Group 1 as the compositor lays it out (compose.rs, colour_layout).
    let mut entries = vec![
        buffer_entry(0, true),
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ];
    for (binding, data) in (2..).zip(&op.textures) {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: match data.dimension {
                    Dimension::D2 => wgpu::TextureViewDimension::D2,
                    Dimension::D3 => wgpu::TextureViewDimension::D3,
                },
                multisampled: false,
            },
            count: None,
        });
    }
    let colour_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ocio test colour"),
        entries: &entries,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ocio test"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ocio test"),
                bind_group_layouts: &[Some(&io_layout), Some(&colour_layout)],
                immediate_size: 0,
            }),
        ),
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let mut params = op.params.clone();
    if params.is_empty() {
        params.push([0.0; 4]);
    }
    let params = storage(card, &params, wgpu::BufferUsages::STORAGE);
    // The compositor's sampler: linear, clamped to the edges.
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("ocio test"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..wgpu::SamplerDescriptor::default()
    });
    let views: Vec<wgpu::TextureView> = op.textures.iter().map(|t| texture(card, t)).collect();
    let mut bindings = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: params.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(&sampler),
        },
    ];
    for (binding, view) in (2..).zip(&views) {
        bindings.push(wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        });
    }
    let colour = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ocio test colour"),
        layout: &colour_layout,
        entries: &bindings,
    });
    Ready {
        pipeline,
        io_layout,
        colour,
    }
}

/// The program on the card at each of `inputs`, and how long the card took.
fn run(card: &Card, ready: &Ready, inputs: &[[f32; 3]]) -> (Vec<[f32; 3]>, f64) {
    let device = &card.device;
    let padded: Vec<[f32; 4]> = inputs.iter().map(|&[r, g, b]| [r, g, b, 1.0]).collect();
    let input = storage(card, &padded, wgpu::BufferUsages::STORAGE);
    let size = (inputs.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ocio test out"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ocio test readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let io = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ocio test io"),
        layout: &ready.io_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let started = Instant::now();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&ready.pipeline);
        pass.set_bind_group(0, &io, &[]);
        pass.set_bind_group(1, &ready.colour, &[]);
        let groups = inputs.len().div_ceil(64);
        // At most 65535 groups in x: rows of them in y would need the shader to know the width,
        // so the tests stay below 4 million colours a dispatch.
        pass.dispatch_workgroups(u32::try_from(groups).unwrap(), 1, 1);
    }
    card.queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the card finishes");
    let took = started.elapsed().as_secs_f64();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
    card.queue.submit([encoder.finish()]);
    let errors = card.errors.lock().unwrap().clone();
    assert!(errors.is_empty(), "the card refused: {errors:?}");
    let (sent, mapped) = std::sync::mpsc::channel();
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the card finishes");
    mapped.recv().unwrap().expect("the buffer is mapped");
    let bytes = readback
        .slice(..)
        .get_mapped_range()
        .expect("mapped")
        .to_vec();
    let out = bytes
        .chunks(16)
        .map(|texel| {
            [0, 1, 2].map(|c| f32::from_le_bytes(texel[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect();
    (out, took)
}

/// The errors of `gpu` from `cpu`, each channel's: absolute, or, `wide`, relative to the
/// largest channel of the colour where that is above 1 (a channel of an HDR or linear colour is
/// as far off as the colour is bright). Where OCIO itself gives no finite value the card is not
/// held to one. The largest, the 99.9th percentile, and where the largest is.
fn errors(cpu: &[[f32; 3]], gpu: &[[f32; 3]], wide: bool) -> (f64, f64, usize) {
    let mut all = Vec::with_capacity(cpu.len() * 3);
    let mut worst = (0.0, 0);
    for (i, (c, g)) in cpu.iter().zip(gpu).enumerate() {
        let size = c
            .iter()
            .map(|v| f64::from(v.abs()))
            .filter(|v| v.is_finite())
            .fold(1.0, f64::max);
        for k in 0..3 {
            let (c, g) = (f64::from(c[k]), f64::from(g[k]));
            if !c.is_finite() {
                continue;
            }
            let error = if g.is_finite() {
                (g - c).abs() / if wide { size } else { 1.0 }
            } else {
                f64::INFINITY
            };
            if error > worst.0 {
                worst = (error, i);
            }
            all.push(error);
        }
    }
    all.sort_by(f64::total_cmp);
    let p999 = all.get(all.len() * 999 / 1000).copied().unwrap_or(0.0);
    (worst.0, p999, worst.1)
}

/// What every colour but one in a thousand is held to.
const TOLERANCE: f64 = 1e-3;
/// What the rest are held to: the ACES 2.0 output transforms of HDR differ by up to 7e-3 (of
/// PQ's code) in the darkest channel of colours far out of every gamut (a green of 2 with red
/// and blue at 0), the same with tables of 32-bit floats, where the gamut compression is at
/// its steepest; and the curves of HLG and Canon Log that OCIO makes tables of, by a few 1e-3
/// for values beyond what their encodings hold.
const OUTLIERS: f64 = 1e-2;

#[test]
fn translated_shaders_are_ocio_on_the_card() {
    let Some(card) = require_card() else { return };
    let config = Config::columbina().unwrap();
    let cases = common::cases(&config);
    let mut report = String::new();
    let mut failed = Vec::new();
    let mut formats = vec![TexelFormat::Rgba16Float];
    if card.float32_filterable {
        formats.push(TexelFormat::Rgba32Float);
    }
    for (index, case) in cases.iter().enumerate() {
        let inputs = common::inputs(case.linear_in, 4096, 7 + index as u64);
        let mut cpu = inputs.clone();
        case.processor.apply(&mut cpu).unwrap();
        for &format in &formats {
            if std::env::var_os("COLUMBINA_OCIO_TRACE").is_some() {
                eprintln!("{} ({format:?})", case.name);
            }
            let op = case.processor.translated(format).unwrap().unwrap();
            let started = Instant::now();
            let ready = prepare(&card, &op);
            let compiled = started.elapsed().as_secs_f64() * 1000.0;
            let (gpu, _) = run(&card, &ready, &inputs);
            let (error, p999, at) = errors(&cpu, &gpu, case.wide_out);
            let _ = writeln!(
                report,
                "{error:>10.3e} {p999:>10.3e} {compiled:>7.1} ms  {format:?} {} tables  {}",
                op.textures.len(),
                case.name
            );
            if p999 > TOLERANCE || error > OUTLIERS {
                failed.push(format!(
                    "{} ({format:?}): {error:e} (99.9%: {p999:e}) at {:?}: CPU {:?}, card {:?}",
                    case.name, inputs[at], cpu[at], gpu[at]
                ));
            }
        }
    }
    eprintln!("{report}");
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

#[test]
fn baked_processors_are_ocio_to_the_precision_of_their_cube() {
    let Some(card) = require_card() else { return };
    let config = Config::columbina().unwrap();
    // A log input in its cube, and a linear one through the log shaper.
    // (Not into a gamma of video: a colour out of its gamut, below 0 there, is where the power
    // of 1/2.4 is steepest, which no cube follows.)
    let log = config.processor("Apple Log", "ACEScct").unwrap();
    let linear = config
        .display_view(
            "ACES2065-1",
            "Rec.1886 Rec.709 - Display",
            "ACES 2.0 - SDR 100 nits (Rec.709)",
            Direction::Forward,
        )
        .unwrap();
    for (name, processor, shaper, linear_in, tolerance) in [
        (
            "Apple Log → ACEScct",
            &log,
            Shaper::Range { min: 0.0, max: 1.0 },
            false,
            5e-3,
        ),
        ("ACES 2.0 SDR", &linear, Shaper::Log, true, 2e-2),
    ] {
        let op = processor
            .baked(
                &Bake {
                    shaper,
                    ..Bake::default()
                },
                TexelFormat::Rgba16Float,
            )
            .unwrap();
        // Colours of the gamut, near grey: out of it, where a gamut is changed and a curve is
        // steep at 0, a cube cannot follow (which is why it is the fallback).
        let inputs: Vec<[f32; 3]> = common::inputs(linear_in, 4096, 3)
            .into_iter()
            .map(|[r, g, b]| {
                let grey = (r + g + b) / 3.0;
                [r, g, b].map(|v| grey + (v - grey) * 0.2)
            })
            .filter(|c| {
                c.iter()
                    .all(|v| *v >= 0.0 && *v <= if linear_in { 64.0 } else { 1.0 })
            })
            .collect();
        let mut cpu = inputs.clone();
        processor.apply(&mut cpu).unwrap();
        let ready = prepare(&card, &op);
        let (gpu, _) = run(&card, &ready, &inputs);
        let (error, _, at) = errors(&cpu, &gpu, true);
        eprintln!("{name}: baked, {error:e} at {:?}", inputs[at]);
        assert!(error < tolerance, "{name}: {error:e}");
    }
}

/// How long a translated output transform takes on a frame of 4K: `cargo test --release
/// --test gpu -- --ignored how_long`.
#[test]
#[ignore = "a measure, not a test"]
fn how_long_output_transforms_take_on_a_frame_of_4k() {
    let Some(card) = require_card() else { return };
    let config = Config::columbina().unwrap();
    let pixels = 3840 * 2160 / 2;
    let inputs = common::inputs(true, pixels, 5);
    for (name, processor) in [
        (
            "ACES 2.0 SDR (Rec.709)",
            config
                .display_view(
                    "ACEScct",
                    "Rec.1886 Rec.709 - Display",
                    "ACES 2.0 - SDR 100 nits (Rec.709)",
                    Direction::Forward,
                )
                .unwrap(),
        ),
        (
            "ACES 2.0 HDR 1000 (PQ)",
            config
                .display_view(
                    "ACEScct",
                    "Rec.2100-PQ - Display",
                    "ACES 2.0 - HDR 1000 nits (P3 D65)",
                    Direction::Forward,
                )
                .unwrap(),
        ),
        (
            "Apple Log → ACEScct",
            config.processor("Apple Log", "ACEScct").unwrap(),
        ),
        (
            "nothing (the cost of the test's own reads and writes)",
            config.processor("ACEScg", "ACEScg").unwrap(),
        ),
    ] {
        let op = processor
            .translated(TexelFormat::Rgba16Float)
            .unwrap()
            .unwrap();
        let ready = prepare(&card, &op);
        let _ = run(&card, &ready, &inputs[..4096]);
        eprintln!(
            "{name}: {:.2} ms for a frame of 4K",
            dispatches(&card, &ready, &inputs, 10) * 2.0 * 1000.0
        );
    }
}

/// The time of one dispatch over `inputs`, the best of `repeats`, with the buffers made and
/// filled before.
fn dispatches(card: &Card, ready: &Ready, inputs: &[[f32; 3]], repeats: usize) -> f64 {
    let device = &card.device;
    let padded: Vec<[f32; 4]> = inputs.iter().map(|&[r, g, b]| [r, g, b, 1.0]).collect();
    let input = storage(card, &padded, wgpu::BufferUsages::STORAGE);
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ocio test out"),
        size: (inputs.len() * 16) as u64,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let io = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ocio test io"),
        layout: &ready.io_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: input.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the card finishes");
    let mut best = f64::MAX;
    for _ in 0..repeats {
        let started = Instant::now();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(&ready.pipeline);
            pass.set_bind_group(0, &io, &[]);
            pass.set_bind_group(1, &ready.colour, &[]);
            pass.dispatch_workgroups(u32::try_from(inputs.len().div_ceil(64)).unwrap(), 1, 1);
        }
        card.queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the card finishes");
        best = best.min(started.elapsed().as_secs_f64());
    }
    best
}
