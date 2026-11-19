use std::{slice, sync::OnceLock};

use ala::{
    Bounds, GLASS_SHADER, Glass, GlassBlurUniform, GlassRun, GlassUniform, Size, WindowCornerMask,
    glass_level_size, plan_glass, size,
};
use anyhow::{Context, Result};
use windows::Win32::{
    Foundation::RECT,
    Graphics::{Direct3D::*, Direct3D11::*},
};

use super::{
    DirectXGlobalElements, DirectXRendererDevices, DirectXResources, create_constant_buffer,
    create_fragment_shader, create_premultiplied_blend_state, create_sampled_render_texture,
    create_vertex_shader, update_buffer,
};

struct Level {
    texture: ID3D11Texture2D,
    input: Option<ID3D11ShaderResourceView>,
    output: Option<ID3D11RenderTargetView>,
    viewport: D3D11_VIEWPORT,
}

impl Level {
    fn new(device: &ID3D11Device, extent: Size<i32>) -> Result<Self> {
        let (texture, input) =
            create_sampled_render_texture(device, extent.width as u32, extent.height as u32)?;
        let mut output = None;
        unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut output))? };
        Ok(Self {
            texture,
            input,
            output,
            viewport: D3D11_VIEWPORT {
                Width: extent.width as f32,
                Height: extent.height as f32,
                MaxDepth: 1.,
                ..Default::default()
            },
        })
    }
}

pub(super) struct GlassPipeline {
    vertex: ID3D11VertexShader,
    fragment: ID3D11PixelShader,
    blur_vertex: ID3D11VertexShader,
    down: ID3D11PixelShader,
    up: ID3D11PixelShader,
    material: ID3D11Buffer,
    blur: ID3D11Buffer,
    rasterizer: ID3D11RasterizerState,
    blend: ID3D11BlendState,
    levels: Vec<Level>,
    capacity: Size<i32>,
    frame_area: i64,
}

impl GlassPipeline {
    pub(super) fn new(device: &ID3D11Device) -> Result<Self> {
        let shaders = shaders()?;
        let desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: true.into(),
            ScissorEnable: true.into(),
            ..Default::default()
        };
        let mut rasterizer = None;
        unsafe { device.CreateRasterizerState(&desc, Some(&mut rasterizer))? };
        Ok(Self {
            vertex: create_vertex_shader(device, &shaders.vertex)?,
            fragment: create_fragment_shader(device, &shaders.fragment)?,
            blur_vertex: create_vertex_shader(device, &shaders.blur_vertex)?,
            down: create_fragment_shader(device, &shaders.down)?,
            up: create_fragment_shader(device, &shaders.up)?,
            material: create_constant_buffer::<GlassUniform>(device)?.context("glass buffer")?,
            blur: create_constant_buffer::<GlassBlurUniform>(device)?.context("blur buffer")?,
            rasterizer: rasterizer.context("glass rasterizer")?,
            blend: create_premultiplied_blend_state(device)?,
            levels: Vec::new(),
            capacity: Size::default(),
            frame_area: 0,
        })
    }

    fn ensure_levels(&mut self, device: &ID3D11Device, run: &GlassRun) -> Result<()> {
        let padded = |length: i32| (length as u32).next_multiple_of(64) as i32;
        let needed = run.extent.map(padded);
        self.frame_area = self
            .frame_area
            .max(i64::from(needed.width) * i64::from(needed.height));
        if needed.width > self.capacity.width || needed.height > self.capacity.height {
            self.capacity = size(
                self.capacity.width.max(needed.width),
                self.capacity.height.max(needed.height),
            );
            self.levels.clear();
        }
        while self.levels.len() <= run.depth as usize {
            let extent = glass_level_size(self.capacity, self.levels.len() as u32);
            self.levels.push(Level::new(device, extent)?);
        }
        Ok(())
    }

    pub(super) fn finish_frame(&mut self) {
        if i64::from(self.capacity.width) * i64::from(self.capacity.height) > 4 * self.frame_area {
            self.levels.clear();
            self.capacity = Size::default();
        }
        self.frame_area = 0;
    }

    pub(super) fn draw(
        &mut self,
        devices: &DirectXRendererDevices,
        resources: &DirectXResources,
        globals: &DirectXGlobalElements,
        glasses: &[Glass],
        window_mask: Option<WindowCornerMask>,
    ) -> Result<()> {
        let viewport = size(
            resources.viewport.Width as i32,
            resources.viewport.Height as i32,
        );
        let context = &devices.device_context;
        let previous = unsafe { context.RSGetState() }.ok();
        unsafe {
            context.RSSetState(&self.rasterizer);
            context.PSSetSamplers(1, Some(slice::from_ref(&globals.surface_sampler)));
            context.PSSetSamplers(5, Some(slice::from_ref(&globals.surface_sampler)));
        }
        let result = plan_glass(glasses, viewport, window_mask)
            .iter()
            .try_for_each(|run| self.draw_run(devices, resources, run));
        unsafe {
            context.PSSetShaderResources(2, Some(&[None, None]));
            context.PSSetShaderResources(6, Some(&[None]));
            context.OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
            context.RSSetViewports(Some(slice::from_ref(&resources.viewport)));
            context.RSSetState(previous.as_ref());
            context.VSSetConstantBuffers(0, Some(slice::from_ref(&globals.global_params_buffer)));
            context.PSSetConstantBuffers(0, Some(slice::from_ref(&globals.global_params_buffer)));
        }
        result
    }

    fn draw_run(
        &mut self,
        devices: &DirectXRendererDevices,
        resources: &DirectXResources,
        run: &GlassRun,
    ) -> Result<()> {
        let context = &devices.device_context;
        if !run.copies.is_empty() {
            self.ensure_levels(&devices.device, run)?;
            let frame = resources
                .render_target
                .as_ref()
                .context("missing glass backdrop")?;
            unsafe {
                context.PSSetShaderResources(2, Some(&[None, None]));
                context.OMSetRenderTargets(None, None);
                for (region, destination) in &run.copies {
                    let source = D3D11_BOX {
                        left: region.origin.x as u32,
                        top: region.origin.y as u32,
                        front: 0,
                        right: region.right() as u32,
                        bottom: region.bottom() as u32,
                        back: 1,
                    };
                    context.CopySubresourceRegion(
                        &self.levels[0].texture,
                        0,
                        destination.x as u32,
                        destination.y as u32,
                        0,
                        frame,
                        0,
                        Some(&source),
                    );
                }
                context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                context.VSSetShader(&self.blur_vertex, None);
                context.OMSetBlendState(None, None, u32::MAX);
                context.PSSetConstantBuffers(4, Some(&[Some(self.blur.clone())]));
            }
            for pass in &run.passes {
                let target = &self.levels[pass.destination as usize];
                let source = &self.levels[pass.source as usize];
                unsafe {
                    context.OMSetRenderTargets(Some(slice::from_ref(&target.output)), None);
                    context.RSSetViewports(Some(slice::from_ref(&target.viewport)));
                    context.PSSetShaderResources(6, Some(slice::from_ref(&source.input)));
                    context.PSSetShader(if pass.down { &self.down } else { &self.up }, None);
                }
                for (uniform, rect) in &pass.draws {
                    update_buffer(context, &self.blur, slice::from_ref(uniform))?;
                    draw_rect(context, *rect, 3);
                }
                unsafe { context.PSSetShaderResources(6, Some(&[None])) };
            }
        }
        unsafe {
            context.OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
            context.RSSetViewports(Some(slice::from_ref(&resources.viewport)));
            if !run.copies.is_empty() {
                context.PSSetShaderResources(
                    2,
                    Some(&[
                        self.levels[0].input.clone(),
                        self.levels.get(1).and_then(|level| level.input.clone()),
                    ]),
                );
            }
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            context.VSSetShader(&self.vertex, None);
            context.PSSetShader(&self.fragment, None);
            context.VSSetConstantBuffers(0, Some(&[Some(self.material.clone())]));
            context.PSSetConstantBuffers(0, Some(&[Some(self.material.clone())]));
        }
        for (uniform, rect) in &run.draws {
            update_buffer(context, &self.material, slice::from_ref(uniform))?;
            let blend = (!uniform.reads_backdrop()).then_some(&self.blend);
            unsafe { context.OMSetBlendState(blend, None, u32::MAX) };
            draw_rect(context, *rect, uniform.vertex_count());
        }
        Ok(())
    }
}

fn draw_rect(context: &ID3D11DeviceContext, bounds: Bounds<i32>, vertices: u32) {
    let rect = RECT {
        left: bounds.origin.x,
        top: bounds.origin.y,
        right: bounds.right(),
        bottom: bounds.bottom(),
    };
    unsafe {
        context.RSSetScissorRects(Some(&[rect]));
        context.Draw(vertices, 0);
    }
}

struct Shaders {
    vertex: Vec<u8>,
    fragment: Vec<u8>,
    blur_vertex: Vec<u8>,
    down: Vec<u8>,
    up: Vec<u8>,
}

fn shaders() -> Result<&'static Shaders> {
    static SHADERS: OnceLock<Result<Shaders, String>> = OnceLock::new();
    SHADERS
        .get_or_init(|| compile_shaders().map_err(|err| format!("{err:#}")))
        .as_ref()
        .map_err(|err| anyhow::anyhow!("{err}"))
}

fn compile_shaders() -> Result<Shaders> {
    use naga_dx11::{back::hlsl, front::wgsl, valid};
    let module = wgsl::parse_str(GLASS_SHADER)
        .map_err(|err| anyhow::anyhow!("{}", err.emit_to_string(GLASS_SHADER)))?;
    let info = valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
        .validate(&module)?;
    let options = hlsl::Options {
        shader_model: hlsl::ShaderModel::V5_0,
        ..Default::default()
    };
    let mut source = String::new();
    let reflection = hlsl::Writer::new(&mut source, &options).write(&module, &info, None)?;
    let shader = |name: &str, profile| {
        let index = module
            .entry_points
            .iter()
            .position(|entry| entry.name == name)
            .with_context(|| format!("missing glass shader {name}"))?;
        let entry = reflection.entry_point_names[index]
            .as_ref()
            .map_err(|err| anyhow::anyhow!("{err:?}"))?;
        compile(&source, entry, profile).with_context(|| format!("compiling {name}"))
    };
    Ok(Shaders {
        vertex: shader("vs_glass", c"vs_4_1")?,
        fragment: shader("fs_glass", c"ps_4_1")?,
        blur_vertex: shader("vs_glass_blur", c"vs_4_1")?,
        down: shader("fs_glass_down", c"ps_4_1")?,
        up: shader("fs_glass_up", c"ps_4_1")?,
    })
}

fn compile(source: &str, entry: &str, profile: &std::ffi::CStr) -> Result<Vec<u8>> {
    use windows::{Win32::Graphics::Direct3D::Fxc::D3DCompile, core::PCSTR};
    let entry = std::ffi::CString::new(entry)?;
    let mut binary = None;
    let mut errors = None;
    unsafe {
        let result = D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR::null(),
            None,
            None,
            PCSTR(entry.as_ptr().cast()),
            PCSTR(profile.as_ptr().cast()),
            0,
            0,
            &mut binary,
            Some(&mut errors),
        );
        if let Err(error) = result {
            if let Some(errors) = errors {
                let bytes =
                    slice::from_raw_parts(errors.GetBufferPointer().cast(), errors.GetBufferSize());
                anyhow::bail!("{error}: {}", String::from_utf8_lossy(bytes));
            }
            return Err(error.into());
        }
        let binary = binary.context("missing shader bytecode")?;
        Ok(
            slice::from_raw_parts(binary.GetBufferPointer().cast(), binary.GetBufferSize())
                .to_vec(),
        )
    }
}

#[cfg(test)]
#[test]
fn glass_samples_the_windows_frame() -> Result<()> {
    use super::{DirectXDevices, DirectXRenderer};
    use ala::{
        Scene, WindowBackgroundAppearance, check_glass_beside_a_clear_margin,
        check_glass_blurs_the_alpha_of_a_clear_frame, check_glass_hides_the_frame_under_it,
        check_glass_over_a_translucent_frame, check_glass_rendering, check_glass_shadow_rendering,
        check_rim_glass_lights_only_the_edge,
    };
    use windows::{Win32::UI::WindowsAndMessaging::*, core::w};
    let devices = DirectXDevices::new()?;
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!(""),
            WS_POPUP,
            0,
            0,
            64,
            64,
            None,
            None,
            None,
            None,
        )?
    };
    let result = (|| {
        let mut renderer = DirectXRenderer::new(hwnd, &devices, true)?;
        anyhow::ensure!(
            renderer.supports_backdrop_sampling(),
            "glass pipeline unavailable"
        );
        renderer.resize(size(64.into(), 64.into()))?;
        let mut render = |scene: &Scene| {
            renderer.render_to_image(scene, WindowBackgroundAppearance::Transparent)
        };
        check_glass_rendering(&mut render)?;
        check_glass_blurs_the_alpha_of_a_clear_frame(&mut render)?;
        check_glass_over_a_translucent_frame(&mut render)?;
        check_glass_hides_the_frame_under_it(&mut render)?;
        check_glass_beside_a_clear_margin(&mut render)?;
        check_rim_glass_lights_only_the_edge(&mut render)?;
        check_glass_shadow_rendering(render)
    })();
    unsafe { DestroyWindow(hwnd)? };
    result
}
