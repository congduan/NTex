//! egui ↔ vello GPU 桥接：vello 离屏渲染 + egui 回调 blit 上屏。
//!
//! 集成方式（egui-wgpu 0.35 三段式回调）：
//! - [`GpuState`]：eframe 共享 device 上创建一次的常驻资源——
//!   `vello::Renderer`（compute 管线，area AA）、blit 管线（全屏 quad 采样
//!   离屏纹理）与帧间复用的离屏纹理。整体存放在
//!   `Renderer::callback_resources`，prepare/paint 共享；
//! - [`VelloPaint::prepare`]：按显示区物理像素建/复用离屏 Rgba8Unorm 纹理
//!   （STORAGE_BINDING 供 vello compute 写、TEXTURE_BINDING 供采样），
//!   `render_to_texture` 矢量光栅化（缩放/平移已折进 Scene 仿射，分辨率
//!   始终 = 显示分辨率，放大不糊）；
//! - [`VelloPaint::paint`]：在 egui 主 render pass 里 blit 上屏
//!   （绑定组随纹理在 prepare 一并建好——paint 阶段拿不到 device）。
//!
//! 颜色语义：vello 离屏字节为 sRGB 编码（与无头回读管路一致），blit 直通
//! 不做转换；wgpu 抽象了 RGBA/BGRA 字节序，shader 无需 swizzle。

use std::sync::Mutex;

use eframe::egui_wgpu::wgpu;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

/// blit 上屏的 WGSL：4 顶点 triangle-strip 全屏 quad，uv 直采直出。
/// 顶点坐标覆盖整个视口（本回调内自设 scissor 裁剪到绘制矩形）。
const BLIT_WGSL: &str = r#"
struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> Out {
    var xy = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
    );
    var uv = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0),
    );
    var out: Out;
    out.pos = vec4<f32>(xy[vi], 0.0, 1.0);
    out.uv = uv[vi];
    return out;
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs(in: Out) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv);
}
"#;

/// 帧间复用的离屏页纹理（vello compute 写入）+ 采样上屏的绑定组。
struct FrameTex {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    w: u32,
    h: u32,
}

/// 常驻 GPU 资源（App 启动时创建一次，插进 egui renderer 的回调资源仓）。
pub struct GpuState {
    renderer: Renderer,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// 显示区尺寸变化时重建。
    frame: Option<FrameTex>,
}

impl GpuState {
    /// 在 eframe 共享 device 上初始化（`target_format` = egui 表面颜色格式）。
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self, String> {
        let renderer = Renderer::new(
            device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                ..Default::default()
            },
        )
        .map_err(|e| format!("vello 渲染器创建失败：{e}"))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ntex-studio:blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ntex-studio:blit:bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ntex-studio:blit:pll"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ntex-studio:blit:pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ntex-studio:blit:sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            renderer,
            pipeline,
            bind_layout,
            sampler,
            frame: None,
        })
    }

    fn bind_group(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ntex-studio:blit:bg"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// 确保离屏纹理与目标尺寸一致（变化才重建，避免每帧分配）。
    fn ensure_frame(&mut self, device: &wgpu::Device, w: u32, h: u32) -> &FrameTex {
        if self.frame.as_ref().is_some_and(|f| f.w == w && f.h == h) {
            return self.frame.as_ref().expect("尺寸已匹配");
        }
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ntex-studio:page"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // vello compute 管线写 storage；blit 阶段作为采样源。
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.bind_group(device, &view);
        self.frame = Some(FrameTex {
            view,
            bind_group,
            w,
            h,
        });
        self.frame.as_ref().expect("刚插入")
    }
}

/// 一帧预览绘制回调：把（已含显示变换的）Scene 光栅化到离屏纹理并 blit。
pub struct VelloPaint {
    /// 当前页显示 Scene（Arc 跨线程共享给回调；egui 要求回调 Send+Sync）。
    pub scene: std::sync::Arc<Scene>,
    /// 显示区物理像素尺寸（update 侧按回调矩形 × pixels_per_point 预算）。
    pub viewport_px: [u32; 2],
}

impl eframe::egui_wgpu::CallbackTrait for VelloPaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &eframe::egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut eframe::egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<Mutex<GpuState>>() else {
            // 初始化顺序保证存在（Studio::new 注入）；缺失属内部不变量破坏。
            eprintln!("ntex-studio：GpuState 未初始化，跳过本帧");
            return Vec::new();
        };
        // prepare 拿到的是独占引用，免锁直取；锁中毒不 panic（规范），
        // 恢复数据继续。
        let gpu = gpu
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (w, h) = (self.viewport_px[0].max(1), self.viewport_px[1].max(1));
        gpu.ensure_frame(device, w, h);
        // 字段拆分借用：renderer 需 &mut（vello 语义），frame 只读。
        let GpuState {
            renderer, frame, ..
        } = &mut *gpu;
        if let Some(frame) = frame.as_ref() {
            if let Err(err) = renderer.render_to_texture(
                device,
                queue,
                &self.scene,
                &frame.view,
                &RenderParams {
                    base_color: Color::from_rgba8(24, 24, 27, 255),
                    width: w,
                    height: h,
                    antialiasing_method: AaConfig::Area,
                },
            ) {
                // 渲染失败不 panic（引擎契约），跳过本帧并留诊断。
                eprintln!("ntex-studio：vello 渲染失败：{err}");
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &eframe::egui_wgpu::CallbackResources,
    ) {
        let Some(gpu) = resources.get::<Mutex<GpuState>>() else {
            return;
        };
        let gpu = gpu.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(frame) = gpu.frame.as_ref() else {
            return; // prepare 未建纹理（异常路径），静默跳过
        };
        // 裁剪到回调矩形（egui 三段式不自动为回调设 scissor；
        // viewport 已由 egui 保证落在屏幕内）。
        let vp = info.viewport_in_pixels();
        let (ox, oy) = (vp.left_px.max(0) as u32, vp.top_px.max(0) as u32);
        let (sw, sh) = (vp.width_px.max(1) as u32, vp.height_px.max(1) as u32);
        render_pass.set_scissor_rect(ox, oy, sw, sh);
        render_pass.set_pipeline(&gpu.pipeline);
        render_pass.set_bind_group(0, &frame.bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }
}

/// 把页面 base Scene 按显示仿射合成出可直接光栅化的显示 Scene。
///
/// 白纸由本函数显式补画（prims 层不产出页面底矩形，无头管线的白底走
/// vello `base_color`；GUI 是深色工作台，只把纸面区域刷白，纸外保持
/// 画布底色）。页面像素坐标（dpi 下的 prims 坐标）→ 显示物理像素：
/// `p' = p * scale + offset`。全物理像素域计算，与 viewport_px 口径一致。
pub fn compose_display(base: &Scene, scale: f64, offset: (f64, f64), page_px: (f64, f64)) -> Scene {
    let mut scene = Scene::new();
    // 白纸与内容共用同一显示仿射，确保纸面与墨迹严格对齐。
    let affine = Affine::translate((offset.0, offset.1)) * Affine::scale(scale);
    scene.fill(
        Fill::NonZero,
        affine,
        Color::from_rgba8(255, 255, 255, 255),
        None,
        &Rect::new(0.0, 0.0, page_px.0, page_px.1),
    );
    scene.append(base, Some(affine));
    scene
}
