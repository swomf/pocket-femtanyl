mod input;
mod renderer;
mod spritesheet;

use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use cairo::{Context, Format, ImageSurface};
use smithay_client_toolkit::reexports::{
    calloop::{
        EventLoop, LoopHandle, channel,
        timer::{TimeoutAction, Timer},
    },
    calloop_wayland_source::WaylandSource,
    client::{
        Connection, QueueHandle,
        globals::registry_queue_init,
        protocol::{wl_output, wl_shm, wl_surface},
    },
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};

use crate::{
    input::InputEvent,
    renderer::Renderer,
    spritesheet::{Animation, Spritesheet},
};

// see https://raw.githubusercontent.com/Smithay/client-toolkit/v0.20.0/examples/simple_layer.rs

// 24fps based on the mod's characters/femt.json.
// 120fps wastes  my cpu in my experience so
// i use our own timer instead of callbacking every frame
const FRAME_RATE: u64 = 24;

const WIDTH: u32 = 480; // from perlinCamera.lua
const HEIGHT: u32 = 360; // from perlinCamera.lua
const MARGIN: i32 = 24;
const NAMESPACE: &str = "pocket-femtanyl"; // for hyprland lua

struct AnimationState {
    active: Animation,
    pressed: Vec<Animation>,
    started_at: Instant,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            active: Animation::Idle,
            pressed: Vec::new(),
            started_at: Instant::now(),
        }
    }
}

impl AnimationState {
    fn set_pressed(&mut self, animation: Animation, pressed: bool) {
        if pressed {
            if !self.pressed.contains(&animation) {
                self.pressed.push(animation);
            }
        } else {
            self.pressed.retain(|candidate| *candidate != animation);
        }

        let next = self.pressed.last().copied().unwrap_or(Animation::Idle);
        if next != self.active {
            self.active = next;
            self.started_at = Instant::now();
        }
    }
}

struct Overlay {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    layer: LayerSurface,

    width: u32,
    height: u32,
    configured: bool,

    renderer: Renderer,
    anim: AnimationState,

    // cache ->recreate only when the surface is resized
    cairo_surface: ImageSurface,
    cairo_size: (i32, i32),
}

fn main() -> Result<()> {
    let spritesheet = Spritesheet::embedded();
    let renderer = Renderer::new(spritesheet).context("failed to prepare anim frames")?;

    let conn = Connection::connect_to_env().context("could not connect to a wayland compositor")?;
    let (globals, event_queue) = registry_queue_init(&conn).context("wayland registry init")?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
    let layer_shell = LayerShell::bind(&globals, &qh).context("wlr layer shell not available")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm not available")?;

    let surface = compositor.create_surface(&qh);

    // empty input region => the overlay is click-through.
    let region = Region::new(&compositor).context("could not create input region")?;
    surface.set_input_region(Some(region.wl_region()));

    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some(NAMESPACE), None);
    layer.set_anchor(Anchor::BOTTOM | Anchor::RIGHT);
    layer.set_margin(0, MARGIN, MARGIN, 0);
    layer.set_size(WIDTH, HEIGHT);
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // comment from v0.20.0/examples/simple_layer.rs:
    // In order for the layer surface to be mapped, we need to perform an initial commit with no attached
    // buffer. For more info, see WaylandSurface::commit
    //
    // The compositor will respond with an initial configure that we can then use to present to the layer
    layer.commit();

    let pool =
        SlotPool::new((WIDTH * HEIGHT * 4) as usize, &shm).context("failed to create shm pool")?;

    let mut overlay = Overlay {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        layer,
        width: WIDTH,
        height: HEIGHT,
        configured: false,
        renderer,
        anim: AnimationState::default(),
        cairo_surface: ImageSurface::create(Format::ARgb32, 1, 1)?,
        cairo_size: (0, 0),
    };

    let mut event_loop: EventLoop<Overlay> =
        EventLoop::try_new().context("failed to create event loop")?;
    let handle = event_loop.handle();

    WaylandSource::new(conn, event_queue)
        .insert(handle.clone())
        .map_err(|error| anyhow::anyhow!("failed to insert wayland source: {error}"))?;

    setup_redraw_timer(&handle);
    setup_hyprland_input(&handle);

    event_loop
        .run(None, &mut overlay, |_| {})
        .context("event loop error")?;
    Ok(())
}

// avoid fps redrawslop to save cpu
fn setup_redraw_timer(handle: &LoopHandle<'static, Overlay>) {
    let interval = Duration::from_micros(1_000_000 / FRAME_RATE);
    handle
        .insert_source(Timer::from_duration(interval), move |_, _, overlay| {
            if let Err(error) = overlay.draw() {
                eprintln!("render failed: {error:#}");
            }
            TimeoutAction::ToDuration(interval)
        })
        .expect("failed to setup redraw timer");
}

// Bridge the Hyprland event thread into the event loop via a channel.
fn setup_hyprland_input(handle: &LoopHandle<'static, Overlay>) {
    let (sender, source) = channel::channel::<InputEvent>();
    handle
        .insert_source(source, |event, _, overlay| {
            if let channel::Event::Msg(event) = event {
                overlay.anim.set_pressed(event.animation, event.pressed);
            }
        })
        .expect("failed to setup input source");
    input::spawn_hyprland_events(sender);
}

impl Overlay {
    fn draw(&mut self) -> Result<()> {
        if !self.configured {
            return Ok(());
        }
        let width = self.width as i32;
        let height = self.height as i32;

        if self.cairo_size != (width, height) {
            self.cairo_surface = ImageSurface::create(Format::ARgb32, width, height)?;
            self.cairo_size = (width, height);
        }

        {
            let context = Context::new(&self.cairo_surface)?;
            let elapsed = self.anim.started_at.elapsed();
            self.renderer
                .render(&context, width, height, self.anim.active, elapsed)?;
        }
        self.cairo_surface.flush();

        let stride = width * 4;
        let cairo_stride = self.cairo_surface.stride();
        let row_bytes = (width * 4) as usize;

        let (buffer, canvas) = self
            .pool
            .create_buffer(width, height, stride, wl_shm::Format::Argb8888)
            .context("failed to create shm buffer")?;

        {
            // cairo ARGB32 is premultiplied native-endian, identical to
            // wl_shm Argb8888, so this is a straight per-row copy.
            let data = self.cairo_surface.data()?;
            for row in 0..height as usize {
                let src = row * cairo_stride as usize;
                let dst = row * stride as usize;
                canvas[dst..dst + row_bytes].copy_from_slice(&data[src..src + row_bytes]);
            }
        }

        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, width, height);
        buffer
            .attach_to(surface)
            .context("failed to attach buffer")?;
        self.layer.commit();
        Ok(())
    }
}

impl CompositorHandler for Overlay {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        // no-op here since redraws are driven by the timer not frame callbacks.
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for Overlay {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        std::process::exit(0);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if configure.new_size.0 != 0 {
            self.width = configure.new_size.0;
        }
        if configure.new_size.1 != 0 {
            self.height = configure.new_size.1;
        }
        self.configured = true;
    }
}

impl ShmHandler for Overlay {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl OutputHandler for Overlay {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for Overlay {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Overlay);
delegate_shm!(Overlay);
delegate_layer!(Overlay);
delegate_output!(Overlay);
delegate_registry!(Overlay);
