use std::num::NonZeroU32;

use glow::HasContext;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext, Version};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use crate::PixelRect;
use crate::present::{Frame, SceneImage};

const VERT: &str = r#"#version 330 core
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const DOWN_FRAG: &str = r#"#version 330 core
in vec2 v_uv;
uniform sampler2D u_scene;
out vec4 o;
void main() {
    o = texture(u_scene, vec2(v_uv.x, 1.0 - v_uv.y));
}
"#;

const BLUR_FRAG: &str = r#"#version 330 core
in vec2 v_uv;
uniform sampler2D u_src;
uniform vec2 u_dir;
out vec4 o;
void main() {
    vec4 c = texture(u_src, v_uv) * 0.2270270270;
    c += texture(u_src, v_uv + u_dir * 1.3846153846) * 0.3162162162;
    c += texture(u_src, v_uv - u_dir * 1.3846153846) * 0.3162162162;
    c += texture(u_src, v_uv + u_dir * 3.2307692308) * 0.0702702703;
    c += texture(u_src, v_uv - u_dir * 3.2307692308) * 0.0702702703;
    o = c;
}
"#;

const COMPOSITE_FRAG: &str = r#"#version 330 core
in vec2 v_uv;
uniform sampler2D u_ui;
uniform sampler2D u_glass;
uniform sampler2D u_scene;
uniform sampler2D u_blur;
uniform bool u_has_scene;
uniform vec3 u_solid;
uniform float u_brightness;
uniform float u_saturation;
uniform float u_noise;
out vec4 o;
void main() {
    ivec2 size = textureSize(u_ui, 0);
    ivec2 p = ivec2(gl_FragCoord.xy);
    p.y = size.y - 1 - p.y;
    vec4 ui = texelFetch(u_ui, p, 0);
    float glass = texelFetch(u_glass, p, 0).r;
    vec3 base = u_solid;
    if (u_has_scene) {
        vec3 scene = texture(u_scene, vec2(v_uv.x, 1.0 - v_uv.y)).rgb;
        vec3 blur = texture(u_blur, v_uv).rgb * u_brightness;
        float luma = dot(blur, vec3(0.2126, 0.7152, 0.0722));
        blur = clamp(mix(vec3(luma), blur, u_saturation), 0.0, 1.0);
        base = mix(scene, blur, glass);
    }
    vec3 c = ui.rgb + (1.0 - ui.a) * base;
    if (u_noise > 0.0) {
        float n = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);
        c = mix(c, vec3(n), u_noise);
    }
    o = vec4(c, 1.0);
}
"#;

struct Target {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
}

#[derive(Clone, Copy, PartialEq)]
struct BlurKey {
    generation: u64,
    size: (u32, u32),
    low: (u32, u32),
    passes: u32,
    spread: f32,
}

pub(crate) struct Gl {
    gl: glow::Context,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    vao: glow::VertexArray,
    down: glow::Program,
    blur: glow::Program,
    blur_dir: Option<glow::UniformLocation>,
    composite: glow::Program,
    ui: glow::Texture,
    glass: glow::Texture,
    scene: glow::Texture,
    low: [Target; 2],
    size: (u32, u32),
    low_size: (u32, u32),
    scene_generation: Option<u64>,
    blur_key: Option<BlurKey>,
}

pub(crate) fn create(
    event_loop: &ActiveEventLoop,
    attrs: WindowAttributes,
) -> Result<(Window, Gl), (Option<Window>, String)> {
    let template = ConfigTemplateBuilder::new().prefer_hardware_accelerated(Some(true));
    let (window, config) = DisplayBuilder::new()
        .with_window_attributes(Some(attrs))
        .build(event_loop, template, |configs| {
            configs
                .min_by_key(|c| c.num_samples())
                .expect("glutin 只在有可用配置时调用")
        })
        .map_err(|e| (None, e.to_string()))?;
    let Some(window) = window else {
        return Err((None, "没有创建窗口".into()));
    };
    match Gl::new(&window, &config) {
        Ok(gl) => Ok((window, gl)),
        Err(e) => Err((Some(window), e)),
    }
}

impl Gl {
    fn new(window: &Window, config: &glutin::config::Config) -> Result<Gl, String> {
        let raw = window.window_handle().map_err(|e| e.to_string())?.as_raw();
        let display = config.display();
        let attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .build(Some(raw));
        let context = unsafe { display.create_context(config, &attrs) }.map_err(|e| e.to_string())?;
        let surface_attrs = window
            .build_surface_attributes(Default::default())
            .map_err(|e| e.to_string())?;
        let surface = unsafe { display.create_window_surface(config, &surface_attrs) }.map_err(|e| e.to_string())?;
        let context = context.make_current(&surface).map_err(|e| e.to_string())?;
        let _ = surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::MIN));
        let gl = unsafe { glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) };
        unsafe {
            let vao = gl.create_vertex_array()?;
            let down = link(&gl, DOWN_FRAG)?;
            let blur = link(&gl, BLUR_FRAG)?;
            let composite = link(&gl, COMPOSITE_FRAG)?;
            gl.use_program(Some(down));
            gl.uniform_1_i32(gl.get_uniform_location(down, "u_scene").as_ref(), 0);
            gl.use_program(Some(blur));
            gl.uniform_1_i32(gl.get_uniform_location(blur, "u_src").as_ref(), 0);
            let blur_dir = gl.get_uniform_location(blur, "u_dir");
            gl.use_program(Some(composite));
            for (name, unit) in [("u_ui", 0), ("u_glass", 1), ("u_scene", 2), ("u_blur", 3)] {
                gl.uniform_1_i32(gl.get_uniform_location(composite, name).as_ref(), unit);
            }
            let ui = texture(&gl, glow::NEAREST, glow::NEAREST)?;
            let glass = texture(&gl, glow::NEAREST, glow::NEAREST)?;
            let scene = texture(&gl, glow::LINEAR_MIPMAP_LINEAR, glow::LINEAR)?;
            let low = [target(&gl)?, target(&gl)?];
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            Ok(Gl {
                gl,
                surface,
                context,
                vao,
                down,
                blur,
                blur_dir,
                composite,
                ui,
                glass,
                scene,
                low,
                size: (0, 0),
                low_size: (0, 0),
                scene_generation: None,
                blur_key: None,
            })
        }
    }

    pub(crate) fn present(&mut self, f: &Frame) -> Result<(), String> {
        let (Some(w), Some(h)) = (NonZeroU32::new(f.width), NonZeroU32::new(f.height)) else {
            return Ok(());
        };
        assert_eq!(
            f.rgba.len(),
            f.width as usize * f.height as usize * 4,
            "画布尺寸与窗口不一致"
        );
        assert_eq!(
            f.glass.len(),
            f.width as usize * f.height as usize,
            "玻璃通道尺寸与窗口不一致"
        );
        let gl = &self.gl;
        unsafe {
            if self.size != (f.width, f.height) {
                self.surface.resize(&self.context, w, h);
                self.size = (f.width, f.height);
                alloc(gl, self.ui, glow::RGBA8, glow::RGBA, f.width, f.height, Some(f.rgba));
                alloc(gl, self.glass, glow::R8, glow::RED, f.width, f.height, Some(f.glass));
            } else {
                for r in f.damage {
                    let r = r.clamp(f.width, f.height);
                    upload(gl, self.ui, glow::RGBA, 4, f.rgba, f.width, r);
                    upload(gl, self.glass, glow::RED, 1, f.glass, f.width, r);
                }
            }
            if let Some(s) = &f.scene
                && self.scene_generation != Some(s.generation)
            {
                self.scene_generation = Some(s.generation);
                alloc(gl, self.scene, glow::RGBA8, glow::RGBA, s.width, s.height, Some(s.rgba));
                gl.bind_texture(glow::TEXTURE_2D, Some(self.scene));
                gl.generate_mipmap(glow::TEXTURE_2D);
            }
            if let Some(s) = &f.scene {
                self.update_blur(s, f);
            }

            let gl = &self.gl;
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            gl.viewport(0, 0, f.width as i32, f.height as i32);
            gl.use_program(Some(self.composite));
            let loc = |name: &str| gl.get_uniform_location(self.composite, name);
            gl.uniform_1_i32(loc("u_has_scene").as_ref(), f.scene.is_some() as i32);
            let [r, g, b, _] = f.background;
            gl.uniform_3_f32(
                loc("u_solid").as_ref(),
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
            );
            gl.uniform_1_f32(loc("u_brightness").as_ref(), f.blur.brightness);
            gl.uniform_1_f32(loc("u_saturation").as_ref(), f.blur.saturation);
            gl.uniform_1_f32(loc("u_noise").as_ref(), f.noise);
            for (unit, tex) in [self.ui, self.glass, self.scene, self.low[0].tex]
                .into_iter()
                .enumerate()
            {
                gl.active_texture(glow::TEXTURE0 + unit as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        self.surface.swap_buffers(&self.context).map_err(|e| e.to_string())
    }

    unsafe fn update_blur(&mut self, scene: &SceneImage, f: &Frame) {
        let d = f.blur.downsample.max(1);
        let low = (f.width.div_ceil(d), f.height.div_ceil(d));
        let key = BlurKey {
            generation: scene.generation,
            size: (f.width, f.height),
            low,
            passes: f.blur.passes,
            spread: f.blur.spread,
        };
        if self.blur_key == Some(key) {
            return;
        }
        self.blur_key = Some(key);
        let gl = &self.gl;
        unsafe {
            if self.low_size != low {
                self.low_size = low;
                for t in &self.low {
                    alloc(gl, t.tex, glow::RGBA8, glow::RGBA, low.0, low.1, None);
                }
            }
            gl.viewport(0, 0, low.0 as i32, low.1 as i32);
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.low[0].fbo));
            gl.use_program(Some(self.down));
            gl.bind_texture(glow::TEXTURE_2D, Some(self.scene));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            gl.use_program(Some(self.blur));
            let (lw, lh) = (low.0 as f32, low.1 as f32);
            for _ in 0..f.blur.passes {
                for (from, to, dir) in [(0, 1, (f.blur.spread / lw, 0.0)), (1, 0, (0.0, f.blur.spread / lh))] {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.low[to].fbo));
                    gl.bind_texture(glow::TEXTURE_2D, Some(self.low[from].tex));
                    gl.uniform_2_f32(self.blur_dir.as_ref(), dir.0, dir.1);
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
            }
        }
    }
}

unsafe fn link(gl: &glow::Context, frag: &str) -> Result<glow::Program, String> {
    unsafe {
        let prog = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, src) in [(glow::VERTEX_SHADER, VERT), (glow::FRAGMENT_SHADER, frag)] {
            let sh = gl.create_shader(kind)?;
            gl.shader_source(sh, src);
            gl.compile_shader(sh);
            if !gl.get_shader_compile_status(sh) {
                return Err(format!("着色器编译失败：{}", gl.get_shader_info_log(sh)));
            }
            gl.attach_shader(prog, sh);
            shaders.push(sh);
        }
        gl.link_program(prog);
        if !gl.get_program_link_status(prog) {
            return Err(format!("着色器链接失败：{}", gl.get_program_info_log(prog)));
        }
        for sh in shaders {
            gl.detach_shader(prog, sh);
            gl.delete_shader(sh);
        }
        Ok(prog)
    }
}

unsafe fn texture(gl: &glow::Context, min: u32, mag: u32) -> Result<glow::Texture, String> {
    unsafe {
        let tex = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, mag as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        Ok(tex)
    }
}

unsafe fn target(gl: &glow::Context) -> Result<Target, String> {
    unsafe {
        let tex = texture(gl, glow::LINEAR, glow::LINEAR)?;
        alloc(gl, tex, glow::RGBA8, glow::RGBA, 1, 1, None);
        let fbo = gl.create_framebuffer()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        Ok(Target { fbo, tex })
    }
}

unsafe fn alloc(
    gl: &glow::Context,
    tex: glow::Texture,
    internal: u32,
    format: u32,
    w: u32,
    h: u32,
    data: Option<&[u8]>,
) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            internal as i32,
            w as i32,
            h as i32,
            0,
            format,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(data),
        );
    }
}

unsafe fn upload(
    gl: &glow::Context,
    tex: glow::Texture,
    format: u32,
    bpp: usize,
    data: &[u8],
    stride: u32,
    r: PixelRect,
) {
    if r.is_empty() {
        return;
    }
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, stride as i32);
        let start = (r.y as usize * stride as usize + r.x as usize) * bpp;
        gl.tex_sub_image_2d(
            glow::TEXTURE_2D,
            0,
            r.x as i32,
            r.y as i32,
            r.width as i32,
            r.height as i32,
            format,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(&data[start..])),
        );
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
    }
}
