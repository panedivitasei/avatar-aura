// GPU resources: pipelines, per-mesh buffers, per-material bind groups, render targets and readback.

use avatar_export::avatar::Mesh;
use avatar_export::rig::Rig;
use avatar_export::scene::Material;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use image::RgbaImage;
use wgpu::util::DeviceExt;

use crate::{Result, ViewError};

pub const SAMPLE_COUNT: u32 = 4;
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

const MESH_WGSL: &str = concat!(
    include_str!("shaders/common.wgsl"),
    include_str!("shaders/mesh.wgsl")
);
const BACKGROUND_WGSL: &str = concat!(
    include_str!("shaders/common.wgsl"),
    include_str!("shaders/background.wgsl")
);
const CONTACT_WGSL: &str = concat!(
    include_str!("shaders/common.wgsl"),
    include_str!("shaders/contact.wgsl")
);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct FrameUniform {
    pub view_proj: [[f32; 4]; 4],
    pub eye: [f32; 4],
    pub params: [f32; 4],
    pub contact: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    flags: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [u8; 4],
    joints: [u16; 4],
    weights: [f32; 4],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![
    0 => Float32x3,
    1 => Float32x3,
    2 => Float32x2,
    3 => Unorm8x4,
    4 => Uint16x4,
    5 => Float32x4,
];

pub fn frame_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("avatar-view frame"),
        size: std::mem::size_of::<FrameUniform>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub fn palette_buffer(device: &wgpu::Device, palette: &[Mat4]) -> wgpu::Buffer {
    let data: &[Mat4] = if palette.is_empty() {
        &[Mat4::IDENTITY]
    } else {
        palette
    };
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("avatar-view palette"),
        contents: bytemuck::cast_slice(&palette_bytes(data)),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    })
}

/// Column-major floats in the storage buffer layout.
pub fn palette_bytes(palette: &[Mat4]) -> Vec<[f32; 16]> {
    palette.iter().map(Mat4::to_cols_array).collect()
}

pub struct Pipelines {
    frame_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    color_format: wgpu::TextureFormat,
    depth: bool,
    /// Indexed by `blended * 2 + double_sided`.
    meshes: [wgpu::RenderPipeline; 4],
    pub background: wgpu::RenderPipeline,
    pub contact: wgpu::RenderPipeline,
}

impl Pipelines {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, depth: bool) -> Self {
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("avatar-view frame"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("avatar-view material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("avatar-view diffuse"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let frame_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("avatar-view frame only"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("avatar-view mesh"),
            bind_group_layouts: &[Some(&frame_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let module = |label: &str, source: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };
        let mesh_module = module("avatar-view mesh", MESH_WGSL);
        let background_module = module("avatar-view background", BACKGROUND_WGSL);
        let contact_module = module("avatar-view contact", CONTACT_WGSL);
        let depth_state = |write: bool, compare: wgpu::CompareFunction| {
            depth.then_some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            })
        };
        let multisample = wgpu::MultisampleState {
            count: SAMPLE_COUNT,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        };
        let mesh_pipeline = |blended: bool, double_sided: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("avatar-view mesh"),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout.clone())],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: (!double_sided).then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: depth_state(!blended, wgpu::CompareFunction::LessEqual),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &mesh_module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: blended.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let meshes = [
            mesh_pipeline(false, false),
            mesh_pipeline(false, true),
            mesh_pipeline(true, false),
            mesh_pipeline(true, true),
        ];
        let screen_pipeline = |label: &str,
                               module: &wgpu::ShaderModule,
                               blend: Option<wgpu::BlendState>,
                               depth_compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&frame_only),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: depth_state(false, depth_compare),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let background = screen_pipeline(
            "avatar-view background",
            &background_module,
            None,
            wgpu::CompareFunction::Always,
        );
        let contact = screen_pipeline(
            "avatar-view contact",
            &contact_module,
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::CompareFunction::LessEqual,
        );
        Self {
            frame_layout,
            material_layout,
            sampler,
            color_format,
            depth,
            meshes,
            background,
            contact,
        }
    }

    pub fn mesh(&self, blended: bool, double_sided: bool) -> &wgpu::RenderPipeline {
        &self.meshes[usize::from(blended) * 2 + usize::from(double_sided)]
    }

    pub fn frame_group(
        &self,
        device: &wgpu::Device,
        frame: &wgpu::Buffer,
        palette: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("avatar-view frame"),
            layout: &self.frame_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: palette.as_entire_binding(),
                },
            ],
        })
    }

    fn material_group(
        &self,
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        texture: &wgpu::Texture,
    ) -> wgpu::BindGroup {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("avatar-view material"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }
}

/// Multisampled colour and depth attachments for one target size.
pub struct Targets {
    pub size: (u32, u32),
    pub msaa: Option<wgpu::TextureView>,
    pub depth: Option<wgpu::TextureView>,
}

impl Targets {
    pub fn new(device: &wgpu::Device, pipelines: &Pipelines, width: u32, height: u32) -> Self {
        let attachment = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: SAMPLE_COUNT,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let color_format = pipelines.color_format;
        Self {
            size: (width, height),
            msaa: (SAMPLE_COUNT > 1).then(|| attachment("avatar-view msaa", color_format)),
            depth: pipelines
                .depth
                .then(|| attachment("avatar-view depth", DEPTH_FORMAT)),
        }
    }
}

/// One mesh's GPU buffers plus the CPU copy used for posed bounds.
pub struct GpuMesh {
    pub material: usize,
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub index_count: u32,
    cpu: Vec<Vertex>,
}

impl GpuMesh {
    /// `None` for meshes without triangles.
    pub fn new(device: &wgpu::Device, rig: &Rig, mesh: &Mesh, colors: &[u8]) -> Result<Option<Self>> {
        let n = mesh.positions.len();
        if n == 0 || mesh.indices.is_empty() {
            return Ok(None);
        }
        if let Some(&bad) = mesh.indices.iter().find(|&&i| i as usize >= n) {
            return Err(ViewError::Mesh {
                mesh: mesh.name.clone(),
                reason: format!("index {bad} outside {n} vertices"),
            });
        }
        let mut cpu = Vec::with_capacity(n);
        for i in 0..n {
            let mut joints = [0u16; 4];
            let mut weights = [0f32; 4];
            let influences = rig.vertex_influences(&mesh.joints[i], &mesh.weights[i]);
            for (k, (bone, weight)) in influences.into_iter().take(4).enumerate() {
                joints[k] = u16::try_from(bone).unwrap_or(0);
                weights[k] = weight as f32;
            }
            let color = colors
                .get(i * 4..i * 4 + 4)
                .map_or([255; 4], |c| [c[0], c[1], c[2], c[3]]);
            cpu.push(Vertex {
                position: mesh.positions[i].map(|v| v as f32),
                normal: mesh.normals[i].map(|v| v as f32),
                uv: mesh.uvs[i].map(|v| v as f32),
                color,
                joints,
                weights,
            });
        }
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("avatar-view vertices"),
            contents: bytemuck::cast_slice(&cpu),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("avatar-view indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Ok(Some(Self {
            material: mesh.material,
            vertices,
            indices,
            index_count: mesh.indices.len() as u32,
            cpu,
        }))
    }

    /// Positions skinned on the CPU with the same four-weight blend as the vertex shader.
    pub fn skinned_positions<'a>(&'a self, palette: &'a [Mat4]) -> impl Iterator<Item = Vec3> + 'a {
        self.cpu.iter().map(move |v| {
            let p = Vec3::from(v.position);
            let mut out = Vec3::ZERO;
            for (j, w) in v.joints.iter().zip(v.weights) {
                if w != 0.0 {
                    let m = palette.get(usize::from(*j)).unwrap_or(&Mat4::IDENTITY);
                    out += m.transform_point3(p) * w;
                }
            }
            out
        })
    }
}

/// A material's bind groups: the scene texture and an optional expression override.
pub struct GpuMaterial {
    pub blended: bool,
    pub double_sided: bool,
    uniform: wgpu::Buffer,
    base: wgpu::BindGroup,
    replacement: Option<wgpu::BindGroup>,
}

impl GpuMaterial {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        material: &Material,
        image: Option<&RgbaImage>,
    ) -> Self {
        let blended = material.has_alpha && !material.alpha_mask;
        let flags = [
            f32::from(u8::from(material.alpha_mask)),
            f32::from(u8::from(material.double_sided)),
            f32::from(u8::from(blended)),
            0.0,
        ];
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("avatar-view material"),
            contents: bytemuck::bytes_of(&MaterialUniform { flags }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let white = RgbaImage::from_pixel(1, 1, image::Rgba([255; 4]));
        let texture = upload_texture(device, queue, image.unwrap_or(&white));
        let base = pipelines.material_group(device, &uniform, &texture);
        Self {
            blended,
            double_sided: material.double_sided,
            uniform,
            base,
            replacement: None,
        }
    }

    pub fn set_override(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &Pipelines,
        image: &RgbaImage,
    ) {
        let texture = upload_texture(device, queue, image);
        self.replacement = Some(pipelines.material_group(device, &self.uniform, &texture));
    }

    pub fn clear_override(&mut self) {
        self.replacement = None;
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        self.replacement.as_ref().unwrap_or(&self.base)
    }
}

/// sRGB texture with a full mip chain built by successive triangle-filter halving.
fn upload_texture(device: &wgpu::Device, queue: &wgpu::Queue, image: &RgbaImage) -> wgpu::Texture {
    let (width, height) = (image.width().max(1), image.height().max(1));
    let levels = 32 - width.max(height).leading_zeros();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("avatar-view diffuse"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: TEXTURE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut level = image.clone();
    for mip in 0..levels {
        let (w, h) = level.dimensions();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            level.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        if mip + 1 < levels {
            level = image::imageops::resize(
                &level,
                (w / 2).max(1),
                (h / 2).max(1),
                image::imageops::FilterType::Triangle,
            );
        }
    }
    texture
}

/// Submits `encoder` with a copy of `texture` appended and returns tightly packed rows.
pub fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let row = 4 * width;
    let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("avatar-view readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        let _ = tx.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| ViewError::Readback(e.to_string()))?;
    rx.recv()
        .map_err(|e| ViewError::Readback(e.to_string()))?
        .map_err(|e| ViewError::Readback(e.to_string()))?;
    let mut out = Vec::with_capacity((row * height) as usize);
    {
        let mapped = buffer
            .get_mapped_range(..)
            .map_err(|e| ViewError::Readback(e.to_string()))?;
        for y in 0..height as usize {
            let start = y * padded as usize;
            out.extend_from_slice(&mapped[start..start + row as usize]);
        }
    }
    buffer.unmap();
    Ok(out)
}
