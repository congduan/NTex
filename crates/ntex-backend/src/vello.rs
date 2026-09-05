//! vello GPU 渲染后端（plan.md §10 M8「GPU 渲染」条目）。
//!
//! [`VelloBackend`] 与 [`TinySkiaBackend`](crate::TinySkiaBackend) 共享
//! [`crate::prims`] 的矩形指令遍历，几何完全一致；差异只在光栅化：
//! vello（wgpu compute 管线，area 亚像素 AA）vs 软光栅（整数覆盖盒）。
//! 因此两后端位图可做几何差分对照（见 tests/vello.rs）。
//!
//! 无头渲染管路：instance → adapter → device/queue → `Scene`（矩形填充）
//! → `Renderer::render_to_texture`（Rgba8Unorm + STORAGE_BINDING 纹理）
//! → copy 到 MAP_READ buffer → [`Pixmap`]。
//!
//! - GPU 不可用（无适配器）返回 [`BackendError`]，调用方可回落软光栅；
//! - `use_cpu` 让 vello 管线的粗光栅前阶段走 CPU（调试用，仍需 wgpu 适配器）；
//! - 每次 `render()` 建一次 GPU 上下文（Renderer 含 shader 编译，页间复用）；
//!   CLI 批量场景可接受，编辑器常驻场景后续可提为长生命周期句柄；
//! - 页面尺寸超过 `max_texture_dimension_2d`（常见 8192/16384）时报错，
//!   不 panic（引擎契约：任意畸形输入不 panic）。

use std::sync::mpsc;

use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

use crate::backend::{Backend, BackendError};
use crate::prims::{collect_page, validate_options, PagePrims, RenderOptions};
use crate::raster::Pixmap;

/// vello GPU 后端。
#[derive(Debug, Clone, Copy)]
pub struct VelloBackend {
    use_cpu: bool,
}

impl VelloBackend {
    /// GPU 渲染（默认）。
    pub fn new() -> Self {
        Self { use_cpu: false }
    }

    /// 指定管线执行模式（vello `use_cpu`：粗光栅前阶段走 CPU，调试用）。
    pub fn with_use_cpu(use_cpu: bool) -> Self {
        Self { use_cpu }
    }
}

impl Default for VelloBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// 一次 render 调用生命周期内的 GPU 上下文。
struct GpuContext {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
}

fn init_context(use_cpu: bool) -> Result<GpuContext, BackendError> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .map_err(|_| {
                BackendError("未找到可用 GPU 适配器（可改用 TinySkiaBackend 软光栅）".to_owned())
            })?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|e| BackendError(format!("wgpu 设备初始化失败：{e}")))?;
    let renderer = Renderer::new(
        &device,
        RendererOptions {
            use_cpu,
            antialiasing_support: AaSupport::area_only(),
            ..Default::default()
        },
    )
    .map_err(|e| BackendError(format!("vello 渲染器创建失败：{e}")))?;
    Ok(GpuContext {
        device,
        queue,
        renderer,
    })
}

impl Backend for VelloBackend {
    fn render(
        &self,
        pages: &[ntex_layout::node::BoxNode],
        _fonts: &[ntex_font::FontMetrics],
        opts: &RenderOptions,
    ) -> Result<Vec<Pixmap>, BackendError> {
        validate_options(opts).map_err(BackendError)?;
        let mut ctx = init_context(self.use_cpu)?;
        let max_dim = ctx.device.limits().max_texture_dimension_2d;
        pages
            .iter()
            .map(|page| {
                let prims = collect_page(page, opts);
                if prims.width == 0
                    || prims.height == 0
                    || prims.width > max_dim
                    || prims.height > max_dim
                {
                    return Err(BackendError(format!(
                        "页面像素尺寸 {}×{} 超出纹理上限 {max_dim}（可降低 DPI）",
                        prims.width, prims.height
                    )));
                }
                render_page(&mut ctx, &prims)
            })
            .collect()
    }
}

/// 渲染一页：矩形指令 → Scene → GPU 纹理 → 回读为 [`Pixmap`]。
fn render_page(ctx: &mut GpuContext, prims: &PagePrims) -> Result<Pixmap, BackendError> {
    let mut scene = Scene::new();
    // overlay 在内容之后绘制（独立通道，仅 debug 开启时非空）。
    for r in prims.rects.iter().chain(&prims.debug) {
        // 非正尺寸丢弃（prims 层已过滤，此处双保险）。
        if r.w <= 0.0 || r.h <= 0.0 {
            continue;
        }
        let (cr, cg, cb) = r.color;
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            Color::from_rgba8(cr, cg, cb, 255),
            None,
            &Rect::new(r.x, r.y, r.x + r.w, r.y + r.h),
        );
    }
    let (w, h) = (prims.width, prims.height);
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ntex-backend:vello:page"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        // vello render_to_texture 的目标格式约定（compute 管线写 storage）。
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    ctx.renderer
        .render_to_texture(
            &ctx.device,
            &ctx.queue,
            &scene,
            &view,
            &RenderParams {
                base_color: Color::from_rgba8(255, 255, 255, 255),
                width: w,
                height: h,
                antialiasing_method: AaConfig::Area,
            },
        )
        .map_err(|e| BackendError(format!("GPU 渲染失败：{e}")))?;
    readback(&ctx.device, &ctx.queue, &texture, w, h)
}

/// 纹理 → buffer 回读（行对齐 padding 去除）→ 紧凑 RGBA。
fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    w: u32,
    h: u32,
) -> Result<Pixmap, BackendError> {
    let stride = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ntex-backend:vello:readback"),
        size: stride as u64 * h as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(enc.finish()));
    // 映射完成经 channel 通知；device.poll(Wait) 驱动回调。
    let (tx, rx) = mpsc::channel::<()>();
    let slice = buf.slice(..);
    slice.map_async(wgpu::MapMode::Read, move |_| {
        let _ = tx.send(());
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| BackendError(format!("GPU 回读等待失败：{e}")))?;
    rx.recv()
        .map_err(|_| BackendError("GPU 回读通道中断".to_owned()))?;
    let mapped = slice.get_mapped_range();
    let row_bytes = w as usize * 4;
    let mut data = vec![0u8; row_bytes * h as usize];
    for row in 0..h as usize {
        data[row * row_bytes..(row + 1) * row_bytes]
            .copy_from_slice(&mapped[row * stride as usize..row * stride as usize + row_bytes]);
    }
    drop(mapped);
    buf.unmap();
    Pixmap::from_rgba(w, h, data).ok_or_else(|| BackendError("回读像素长度不符".to_owned()))
}
