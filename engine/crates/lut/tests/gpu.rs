//! The WGSL of the crate on a card, against the reference on the processor: each LUT's body,
//! put in a function as the compositor puts an operation's (`param($P + k)` and `$Tn` replaced
//! alike), run by a compute shader on random colours, beyond the domain too. Within 1e-4 of the
//! reference: of the tables as they are with `Rgba32Float`, of the tables rounded to half floats
//! with `Rgba16Float`.
//!
//! With no card (and no software adapter) there is nothing to run, and the test says so.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};

use columbina_lut::gpu::{Dimension, GpuLut, TexelFormat, TextureData, WGSL};
use columbina_lut::{Domain, Interpolation, Lut, Lut1d, Lut3d};

struct Card {
    device: wgpu::Device,
    queue: wgpu::Queue,
    errors: Arc<Mutex<Vec<String>>>,
}

fn card() -> Option<Card> {
    if !wgpu::Instance::enabled_backend_features().intersects(wgpu::Backends::PRIMARY) {
        return None;
    }
    // FXC, as the engine compiles with: a `dxcompiler.dll` on the PATH may be one that crashes.
    let mut descriptor = wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    };
    descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::Fxc;
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
        apply_limit_buckets: false,
    }))
    .ok()?;
    eprintln!("on {}", adapter.get_info().name);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("columbina-lut test"),
        ..wgpu::DeviceDescriptor::default()
    }))
    .ok()?;
    // Errors are said here and the test fails at the next check: wgpu's own handler panics
    // inside the card's callback, which aborts the process without a word.
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
    })
}

/// Random colours, the same each run, from -0.3 to 1.5.
fn colours(count: usize, seed: u64) -> Vec<[f32; 3]> {
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let unit = ((state >> 40) as f64 / f64::from(1 << 24)) as f32;
        unit * 1.8 - 0.3
    };
    (0..count).map(|_| [next(), next(), next()]).collect()
}

fn random_table(rows: usize, seed: u64) -> Vec<[f32; 3]> {
    colours(rows, seed)
        .into_iter()
        .map(|c| c.map(|v| v * 1.5))
        .collect()
}

/// The body of an operation made a function, as the compositor makes it.
fn shader(op: &GpuLut) -> String {
    let mut body = op.body.replace("$P", "0u");
    for number in (0..op.textures.len()).rev() {
        body = body.replace(&format!("$T{number}"), &format!("colour_texture_{number}"));
    }
    let mut code = String::from(WGSL);
    code.push_str(
        "@group(0) @binding(0) var<storage, read> colour_params: array<vec4<f32>>;\n\
         @group(0) @binding(1) var<storage, read> inputs: array<vec4<f32>>;\n\
         @group(0) @binding(2) var<storage, read_write> outputs: array<vec4<f32>>;\n\
         fn param(index: u32) -> vec4<f32> {\n    return colour_params[index];\n}\n",
    );
    for (number, texture) in op.textures.iter().enumerate() {
        let kind = match texture.dimension {
            Dimension::D2 => "texture_2d<f32>",
            Dimension::D3 => "texture_3d<f32>",
        };
        let _ = writeln!(
            code,
            "@group(0) @binding({}) var colour_texture_{number}: {kind};",
            3 + number
        );
    }
    let _ = write!(
        code,
        "fn colour_op(colour_in: vec3<f32>) -> vec3<f32> {{\n    var rgb = colour_in;\n    {{\n{body}\n    }}\n    return rgb;\n}}\n\
         @compute @workgroup_size(64)\n\
         fn main(@builtin(global_invocation_id) id: vec3<u32>) {{\n\
             if id.x >= arrayLength(&inputs) {{ return; }}\n\
             outputs[id.x] = vec4<f32>(colour_op(inputs[id.x].xyz), 1.0);\n\
         }}\n"
    );
    code
}

fn texture(card: &Card, data: &TextureData) -> wgpu::TextureView {
    let texture = card.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lut"),
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
        label: Some("lut test"),
        size: bytes.len() as u64,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    card.queue.write_buffer(&buffer, 0, &bytes);
    buffer
}

/// The op on the card, at each of `inputs`.
#[allow(clippy::too_many_lines)]
fn run(card: &Card, op: &GpuLut, inputs: &[[f32; 3]]) -> Vec<[f32; 3]> {
    let device = &card.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lut test"),
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
    let mut entries = vec![
        buffer_entry(0, true),
        buffer_entry(1, true),
        buffer_entry(2, false),
    ];
    for (binding, data) in (3..).zip(&op.textures) {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                // As the compositor binds them where they can be: filterable halves.
                sample_type: wgpu::TextureSampleType::Float {
                    filterable: data.format == TexelFormat::Rgba16Float,
                },
                view_dimension: match data.dimension {
                    Dimension::D2 => wgpu::TextureViewDimension::D2,
                    Dimension::D3 => wgpu::TextureViewDimension::D3,
                },
                multisampled: false,
            },
            count: None,
        });
    }
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lut test"),
        entries: &entries,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("lut test"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lut test"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            }),
        ),
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let params = storage(card, &op.params, wgpu::BufferUsages::STORAGE);
    let padded: Vec<[f32; 4]> = inputs.iter().map(|&[r, g, b]| [r, g, b, 1.0]).collect();
    let input = storage(card, &padded, wgpu::BufferUsages::STORAGE);
    let size = (inputs.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lut test out"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lut test readback"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let views: Vec<wgpu::TextureView> = op.textures.iter().map(|t| texture(card, t)).collect();
    let mut bindings = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: params.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: input.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: output.as_entire_binding(),
        },
    ];
    for (binding, view) in (3..).zip(&views) {
        bindings.push(wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        });
    }
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lut test"),
        layout: &layout,
        entries: &bindings,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(u32::try_from(inputs.len().div_ceil(64)).unwrap(), 1, 1);
    }
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
    bytes
        .chunks(16)
        .map(|texel| {
            [0, 1, 2].map(|c| f32::from_le_bytes(texel[c * 4..c * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

fn luts() -> Vec<(&'static str, Lut)> {
    // A shaper as shapers are, a log curve into 0..1 over a wider domain. (A table of random
    // points is no test of the arithmetic: in f32 the input is placed among 1024 points to
    // within 6e-5 of a point, and a random curve is steep enough to make that 1e-4 and more.)
    let shaper = Lut1d::from_fn(1024, Domain::uniform(-0.2, 1.4), |x| {
        x.map(|v| ((v + 0.2) * 8.0 + 1.0).log2() / 12.8_f64.log2() * 0.98 + 0.01)
    })
    .unwrap();
    let long = Lut1d::new(
        Domain {
            min: [-0.1, 0.0, 0.2],
            max: [1.1, 1.0, 1.3],
        },
        Lut1d::from_fn(5000, Domain::default(), |x| x.map(|v| v.powf(1.0 / 2.2)))
            .unwrap()
            .table()
            .to_vec(),
    )
    .unwrap();
    let random = Lut3d::new(17, Domain::default(), random_table(17 * 17 * 17, 11)).unwrap();
    let look = Lut3d::from_fn(33, Domain::uniform(-0.25, 1.25), |[r, g, b]| {
        [
            (0.9 * r + 0.1 * g).max(0.0).powf(0.8),
            (r * g * b).max(0.0).cbrt(),
            (0.2 * r + 0.8 * b).max(0.0).sqrt(),
        ]
    })
    .unwrap();
    vec![
        (
            "a shaper and a random cube",
            Lut::new(None, Some(shaper), Some(random.clone())).unwrap(),
        ),
        ("a random cube", Lut::new(None, None, Some(random)).unwrap()),
        ("a smooth cube", Lut::new(None, None, Some(look)).unwrap()),
        (
            "a 1D table in two rows",
            Lut::new(None, Some(long), None).unwrap(),
        ),
    ]
}

#[test]
fn the_wgsl_reads_a_lut_as_the_reference_does() {
    let Some(card) = card() else {
        eprintln!("no card: the WGSL is not run");
        return;
    };
    let inputs = colours(8192, 99);
    // The corners and edges of the domain, and grey, where branches meet.
    let mut inputs = inputs;
    inputs.extend([
        [0.0; 3],
        [1.0; 3],
        [0.5; 3],
        [1.0, 0.0, 1.0],
        [0.25, 0.25, 0.75],
        [-1.0, 2.0, 0.5],
    ]);
    let mut worst = 0.0_f32;
    for (name, lut) in luts() {
        for format in [TexelFormat::Rgba32Float, TexelFormat::Rgba16Float] {
            let reference = match format {
                TexelFormat::Rgba32Float => lut.clone(),
                TexelFormat::Rgba16Float => lut.rounded_to_half(),
            };
            for interpolation in [Interpolation::Tetrahedral, Interpolation::Trilinear] {
                let op = lut.gpu(interpolation, format);
                let out = run(&card, &op, &inputs);
                for (rgb, got) in inputs.iter().zip(&out) {
                    let want = reference.eval(*rgb, interpolation);
                    for c in 0..3 {
                        let off = (got[c] - want[c]).abs();
                        worst = worst.max(off);
                        assert!(
                            off <= 1e-4,
                            "{name}, {format:?}, {interpolation:?}, at {rgb:?}: {got:?} not {want:?}"
                        );
                    }
                }
            }
        }
    }
    eprintln!("the most that the card and the reference differ: {worst:e}");
}
