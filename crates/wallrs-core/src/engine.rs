use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_dispatch2, delegate_registry,
    output::{OutputHandler, OutputState},
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;
use wallrs_proto::{OutputSelector, PropertyValue, Response};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    backend::ObjectId,
    delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_region, wl_seat, wl_surface},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};

use crate::ipc::{self, IpcError};
use crate::output::OutputSurface;
use wallrs_render::WallpaperRenderer;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Failed to connect to Wayland display: {0}")]
    WaylandConnection(String),

    #[error("Compositor global error: {0}")]
    Global(String),

    #[error(
        "wlr-layer-shell protocol is not supported by the current compositor. wallrs requires a compositor with wlr-layer-shell support (such as Sway, Hyprland, river, labwc, etc.)."
    )]
    LayerShellNotSupported,

    #[error("WGPU adapter not found")]
    AdapterNotFound,

    #[error("Failed to request WGPU device: {0}")]
    DeviceRequest(String),

    #[error("Calloop event loop error: {0}")]
    EventLoop(String),

    #[error("IPC server error: {0}")]
    Ipc(#[from] IpcError),
}

/// Tracks the state of an open toplevel window for automatic fullscreen detection.
pub struct ToplevelData {
    pub handle: ZwlrForeignToplevelHandleV1,
    pub outputs: HashSet<u32>,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub is_fullscreen: bool,
    pub is_maximized: bool,
    pub is_activated: bool,
    pub pending_fullscreen: Option<bool>,
    pub pending_maximized: Option<bool>,
    pub pending_activated: Option<bool>,
    pub outputs_changed: bool,
}

/// Holds all state managed by the Wayland and render event loop.
pub struct EngineState {
    pub qh: QueueHandle<EngineState>,
    pub registry_state: RegistryState,
    pub output_state: OutputState,
    pub seat_state: SeatState,
    pub pointers: Vec<wl_pointer::WlPointer>,
    pub cursor_shape_mgr: Option<WpCursorShapeManagerV1>,
    pub cursor_shape_devices: Vec<WpCursorShapeDeviceV1>,
    pub compositor_state: CompositorState,
    pub layer_shell: LayerShell,
    pub layer: Layer,
    pub wgpu_instance: wgpu::Instance,
    pub wgpu_adapter: wgpu::Adapter,
    pub wgpu_device: wgpu::Device,
    pub wgpu_queue: wgpu::Queue,
    pub outputs: HashMap<ObjectId, OutputSurface>,
    pub renderer_factory: Arc<dyn Fn() -> Box<dyn WallpaperRenderer>>,
    pub audio_handle: Option<wallrs_audio::SpectrumHandle>,
    pub audio_capture: Option<wallrs_audio::AudioCapture>,
    pub max_fps: Option<u32>,
    pub fullscreen_pause: bool,
    pub pause_on_maximized: bool,
    pub allow_audio: bool,
    pub toplevel_manager: Option<ZwlrForeignToplevelManagerV1>,
    pub toplevels: HashMap<ObjectId, ToplevelData>,
    pub exit: bool,
    pub state_snapshot: crate::state::StateSnapshot,
    pub state_path: PathBuf,
    pub restore_state: bool,
}

impl CompositorHandler for EngineState {
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
        qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        if let Some(output) = self.outputs.get_mut(&surface.id()) {
            output.render_frame(&self.wgpu_device, &self.wgpu_queue, qh);
        }
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

impl OutputHandler for EngineState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let name = self.output_state.info(&output).and_then(|info| info.name);
        tracing::info!(output = ?name, "New output detected; creating background layer surface");

        let surface = self.compositor_state.create_surface(qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            qh,
            surface,
            self.layer,
            Some("desktop"),
            Some(&output),
        );

        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer_surface.set_size(0, 0);

        // Apply an empty input region by default so desktop clicks and events pass through
        let region = self.compositor_state.wl_compositor().create_region(qh, ());
        layer_surface.wl_surface().set_input_region(Some(&region));
        region.destroy();

        layer_surface.commit();

        let surface_id = layer_surface.wl_surface().id();
        let mut output_surface = OutputSurface::new(name, output, layer_surface, self.max_fps);
        output_surface.audio_handle = self.audio_handle.clone();
        self.outputs.insert(surface_id, output_surface);
        if self.fullscreen_pause {
            self.update_fullscreen_pause();
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let name = self.output_state.info(&output).and_then(|info| info.name);
        let mut to_restore = None;
        for (id, out) in self.outputs.iter_mut() {
            if out.wl_output == output {
                let had_no_name = out.name.is_none();
                out.name = name.clone();
                if had_no_name && out.configured && name.is_some() {
                    to_restore = Some(id.clone());
                }
                break;
            }
        }
        if let Some(id) = to_restore {
            self.try_restore_output_state(&id);
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let mut to_remove = None;
        for (id, out) in &self.outputs {
            if out.wl_output == output {
                to_remove = Some((id.clone(), out.name.clone()));
                break;
            }
        }
        if let Some((id, name)) = to_remove {
            tracing::info!(output = ?name, "Output destroyed; cleaning up layer surface");
            if let Some(mut out) = self.outputs.remove(&id) {
                out.teardown();
            }
            self.maybe_stop_audio_capture();
        }
    }
}

impl LayerShellHandler for EngineState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface) {
        let surface_id = layer.wl_surface().id();
        if let Some(mut out) = self.outputs.remove(&surface_id) {
            tracing::info!(output = ?out.name, "Layer surface closed by compositor");
            out.teardown();
            self.maybe_stop_audio_capture();
        }
    }

    fn configure(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let surface_id = layer.wl_surface().id();
        let was_configured = self
            .outputs
            .get(&surface_id)
            .map(|o| o.configured)
            .unwrap_or(false);
        if let Some(output) = self.outputs.get_mut(&surface_id) {
            let gpu = crate::output::GpuContext {
                instance: &self.wgpu_instance,
                adapter: &self.wgpu_adapter,
                device: &self.wgpu_device,
                queue: &self.wgpu_queue,
            };
            if let Err(e) = output.handle_configure(
                configure.new_size,
                &gpu,
                conn,
                qh,
                &*self.renderer_factory,
                self.compositor_state.wl_compositor(),
            ) {
                tracing::error!(output = ?output.name, error = ?e, "Failed configuring output surface");
            }
        }

        if !was_configured {
            let is_configured = self
                .outputs
                .get(&surface_id)
                .map(|o| o.configured)
                .unwrap_or(false);
            if is_configured {
                self.try_restore_output_state(&surface_id);
            }
        }
    }
}

impl SeatHandler for EngineState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if let Ok(pointer) = self.seat_state.get_pointer(qh, &seat) {
            if let Some(mgr) = &self.cursor_shape_mgr {
                self.cursor_shape_devices
                    .push(mgr.get_pointer(&pointer, qh, ()));
            }
            self.pointers.push(pointer);
        }
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Ok(pointer) = self.seat_state.get_pointer(qh, &seat)
        {
            if let Some(mgr) = &self.cursor_shape_mgr {
                self.cursor_shape_devices
                    .push(mgr.get_pointer(&pointer, qh, ()));
            }
            self.pointers.push(pointer);
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        _capability: Capability,
    ) {
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
    }
}

impl PointerHandler for EngineState {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if let PointerEventKind::Enter { serial } = event.kind {
                for shape_device in &self.cursor_shape_devices {
                    shape_device.set_shape(serial, Shape::Default);
                }
            }
            if let PointerEventKind::Motion { .. } | PointerEventKind::Enter { .. } = event.kind {
                let surface_id = event.surface.id();
                if let Some(out) = self.outputs.get_mut(&surface_id)
                    && out.width > 0
                    && out.height > 0
                {
                    let norm_x =
                        ((event.position.0 as f32 / out.width as f32) * 2.0 - 1.0).clamp(-1.0, 1.0);
                    let norm_y = ((event.position.1 as f32 / out.height as f32) * 2.0 - 1.0)
                        .clamp(-1.0, 1.0);
                    out.cursor_position = Some((norm_x, norm_y));
                }
            } else if let PointerEventKind::Leave { .. } = event.kind {
                let surface_id = event.surface.id();
                if let Some(out) = self.outputs.get_mut(&surface_id) {
                    out.cursor_position = None;
                }
            }
        }
    }
}

impl ProvidesRegistryState for EngineState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(EngineState);
delegate_dispatch2!(EngineState);
delegate_noop!(EngineState: ignore wl_region::WlRegion);
delegate_noop!(EngineState: ignore WpCursorShapeManagerV1);
delegate_noop!(EngineState: ignore WpCursorShapeDeviceV1);

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for EngineState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { toplevel } => {
                let id = toplevel.id();
                state.toplevels.insert(
                    id,
                    ToplevelData {
                        handle: toplevel,
                        outputs: HashSet::new(),
                        app_id: None,
                        title: None,
                        is_fullscreen: false,
                        is_maximized: false,
                        is_activated: false,
                        pending_fullscreen: None,
                        pending_maximized: None,
                        pending_activated: None,
                        outputs_changed: false,
                    },
                );
            }
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                state.toplevel_manager = None;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(EngineState, ZwlrForeignToplevelManagerV1, [
        0 => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for EngineState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        let id = proxy.id();
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { title } => {
                if let Some(data) = state.toplevels.get_mut(&id) {
                    data.title = Some(title);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::AppId { app_id } => {
                if let Some(data) = state.toplevels.get_mut(&id) {
                    data.app_id = Some(app_id);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputEnter { output } => {
                if let Some(data) = state.toplevels.get_mut(&id)
                    && data.outputs.insert(output.id().protocol_id())
                {
                    data.outputs_changed = true;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputLeave { output } => {
                if let Some(data) = state.toplevels.get_mut(&id)
                    && data.outputs.remove(&output.id().protocol_id())
                {
                    data.outputs_changed = true;
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: state_bytes } => {
                if let Some(data) = state.toplevels.get_mut(&id) {
                    // Enum entries in zwlr_foreign_toplevel_handle_v1::state:
                    // 0 = maximized, 1 = minimized, 2 = activated, 3 = fullscreen
                    let chunks = state_bytes.as_chunks::<4>().0;
                    let is_maximized = chunks.iter().any(|chunk| u32::from_ne_bytes(*chunk) == 0);
                    let is_activated = chunks.iter().any(|chunk| u32::from_ne_bytes(*chunk) == 2);
                    let is_fullscreen = chunks.iter().any(|chunk| u32::from_ne_bytes(*chunk) == 3);
                    data.pending_maximized = Some(is_maximized);
                    data.pending_activated = Some(is_activated);
                    data.pending_fullscreen = Some(is_fullscreen);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                let mut changed = false;
                if let Some(data) = state.toplevels.get_mut(&id) {
                    if data.outputs_changed {
                        data.outputs_changed = false;
                        changed = true;
                    }
                    if let Some(fs) = data.pending_fullscreen.take()
                        && data.is_fullscreen != fs
                    {
                        data.is_fullscreen = fs;
                        changed = true;
                    }
                    if let Some(max) = data.pending_maximized.take()
                        && data.is_maximized != max
                    {
                        data.is_maximized = max;
                        changed = true;
                    }
                    if let Some(act) = data.pending_activated.take()
                        && data.is_activated != act
                    {
                        data.is_activated = act;
                        changed = true;
                    }
                    if changed {
                        tracing::info!(
                            app_id = ?data.app_id,
                            title = ?data.title,
                            is_fullscreen = data.is_fullscreen,
                            is_maximized = data.is_maximized,
                            is_activated = data.is_activated,
                            outputs = ?data.outputs,
                            "Toplevel window state changed"
                        );
                    }
                }
                if changed && state.fullscreen_pause {
                    state.update_fullscreen_pause();
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                if let Some(data) = state.toplevels.remove(&id) {
                    data.handle.destroy();
                    let was_blocking =
                        data.is_fullscreen || (state.pause_on_maximized && data.is_maximized);
                    if was_blocking && state.fullscreen_pause {
                        state.update_fullscreen_pause();
                    }
                }
            }
            _ => {}
        }
    }
}

impl EngineState {
    /// Recalculates fullscreen/maximized pause status for all active outputs.
    pub fn update_fullscreen_pause(&mut self) {
        let mut paused_outputs = HashSet::new();
        for toplevel in self.toplevels.values() {
            let is_blocking =
                toplevel.is_fullscreen || (self.pause_on_maximized && toplevel.is_maximized);
            if is_blocking {
                if toplevel.outputs.is_empty() {
                    for out in self.outputs.values() {
                        paused_outputs.insert(out.wl_output.id().protocol_id());
                    }
                } else {
                    for &out_id in &toplevel.outputs {
                        paused_outputs.insert(out_id);
                    }
                }
            }
        }

        tracing::info!(
            pause_on_maximized = self.pause_on_maximized,
            paused_output_count = paused_outputs.len(),
            "Evaluated fullscreen/maximized pause state across outputs"
        );

        for out in self.outputs.values_mut() {
            let should_pause = paused_outputs.contains(&out.wl_output.id().protocol_id());
            out.set_fullscreen_paused(should_pause, &self.qh);
        }
    }

    /// Ensures PipeWire audio capture is running, starting it on demand if necessary.
    pub fn ensure_audio_capture(&mut self) -> Option<wallrs_audio::SpectrumHandle> {
        if let Some(handle) = &self.audio_handle {
            return Some(handle.clone());
        }

        match wallrs_audio::AudioCapture::start(32) {
            Ok(capture) => {
                let handle = capture.spectrum_handle();
                self.audio_handle = Some(handle.clone());
                self.audio_capture = Some(capture);
                tracing::info!("PipeWire audio capture started on demand (32 frequency bands)");
                Some(handle)
            }
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    "PipeWire audio capture unavailable; running in silent mode"
                );
                None
            }
        }
    }

    /// Stops PipeWire audio capture if no outputs currently have an active audio handle.
    pub fn maybe_stop_audio_capture(&mut self) {
        let any_audio = self.outputs.values().any(|out| out.audio_handle.is_some());
        if !any_audio && self.audio_capture.is_some() {
            tracing::info!("No active audio-reactive outputs; stopping PipeWire audio capture");
            if let Some(mut capture) = self.audio_capture.take() {
                capture.stop();
            }
            self.audio_handle = None;
        }
    }

    /// Records an applied wallpaper into persistent state snapshot and saves it to disk.
    pub fn record_set_wallpaper(&mut self, selector: &OutputSelector, manifest_path: &Path) {
        let abs_path =
            std::fs::canonicalize(manifest_path).unwrap_or_else(|_| manifest_path.to_path_buf());
        let wallpaper_dir = if abs_path.is_file() {
            abs_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or(abs_path)
        } else {
            abs_path
        };

        for out in self.outputs.values() {
            let matches = match selector {
                OutputSelector::All => true,
                OutputSelector::Named(n) => out.name.as_deref() == Some(n.as_str()),
                OutputSelector::Span(names) => out.name.as_ref().is_some_and(|n| names.contains(n)),
            };
            if matches && let Some(name) = &out.name {
                self.state_snapshot.outputs.insert(
                    name.clone(),
                    crate::state::SavedOutputConfig::Wallpaper {
                        path: wallpaper_dir.clone(),
                        muted: out.audio_muted,
                        properties: HashMap::new(),
                    },
                );
            }
        }
        let _ = crate::state::save_state(&self.state_path, &self.state_snapshot);
    }

    /// Records property changes into persistent state and saves to disk.
    pub fn record_set_property(
        &mut self,
        selector: &OutputSelector,
        key: &str,
        value: &PropertyValue,
    ) {
        for out in self.outputs.values() {
            let matches = match selector {
                OutputSelector::All => true,
                OutputSelector::Named(n) => out.name.as_deref() == Some(n.as_str()),
                OutputSelector::Span(names) => out.name.as_ref().is_some_and(|n| names.contains(n)),
            };
            if matches && let Some(name) = &out.name {
                if key == "color" {
                    if let PropertyValue::Color(c) = value {
                        self.state_snapshot.outputs.insert(
                            name.clone(),
                            crate::state::SavedOutputConfig::Color { color: *c },
                        );
                    }
                } else if key == "mute" {
                    if let PropertyValue::Bool(m) = value
                        && let Some(crate::state::SavedOutputConfig::Wallpaper { muted, .. }) =
                            self.state_snapshot.outputs.get_mut(name)
                    {
                        *muted = *m;
                    }
                } else if let Some(crate::state::SavedOutputConfig::Wallpaper {
                    properties, ..
                }) = self.state_snapshot.outputs.get_mut(name)
                {
                    properties.insert(key.to_string(), value.clone());
                }
            }
        }
        let _ = crate::state::save_state(&self.state_path, &self.state_snapshot);
    }

    /// Records a mute toggle change into persistent state and saves to disk.
    pub fn record_mute_change(&mut self, selector: &OutputSelector, new_muted: bool) {
        for out in self.outputs.values() {
            let matches = match selector {
                OutputSelector::All => true,
                OutputSelector::Named(n) => out.name.as_deref() == Some(n.as_str()),
                OutputSelector::Span(names) => out.name.as_ref().is_some_and(|n| names.contains(n)),
            };
            if matches
                && let Some(name) = &out.name
                && let Some(crate::state::SavedOutputConfig::Wallpaper { muted, .. }) =
                    self.state_snapshot.outputs.get_mut(name)
            {
                *muted = new_muted;
            }
        }
        let _ = crate::state::save_state(&self.state_path, &self.state_snapshot);
    }

    /// Attempts to restore a saved wallpaper or color state to an output after it has been configured.
    pub fn try_restore_output_state(&mut self, surface_id: &ObjectId) {
        if !self.restore_state {
            return;
        }
        let output_name = match self.outputs.get(surface_id).and_then(|o| o.name.clone()) {
            Some(n) => n,
            None => return,
        };

        let saved = match self.state_snapshot.outputs.get(&output_name) {
            Some(s) => s.clone(),
            None => return,
        };

        match saved {
            crate::state::SavedOutputConfig::Wallpaper {
                path,
                muted,
                properties,
            } => {
                let manifest_path = if path.is_dir() {
                    path.join("wallpaper.toml")
                } else {
                    path.clone()
                };

                if !manifest_path.exists() {
                    tracing::warn!(
                        output = %output_name,
                        path = %manifest_path.display(),
                        "Saved wallpaper path does not exist; keeping default background"
                    );
                    return;
                }

                tracing::info!(
                    output = %output_name,
                    path = %manifest_path.display(),
                    "Restoring saved wallpaper from session state"
                );

                let selector = OutputSelector::Named(output_name.clone());
                let resp = crate::ipc::apply_wallpaper(self, &selector, &manifest_path);
                if let Response::Ok = resp
                    && let Some(out) = self.outputs.get_mut(surface_id)
                {
                    out.set_muted(muted);
                    for (k, v) in properties {
                        let _ = out.set_property(&k, v, &self.qh);
                    }
                }
            }
            crate::state::SavedOutputConfig::Color { color } => {
                tracing::info!(
                    output = %output_name,
                    color = ?color,
                    "Restoring saved background color from session state"
                );
                if let Some(out) = self.outputs.get_mut(surface_id) {
                    let gpu = crate::output::GpuContext {
                        instance: &self.wgpu_instance,
                        adapter: &self.wgpu_adapter,
                        device: &self.wgpu_device,
                        queue: &self.wgpu_queue,
                    };
                    let solid = Box::new(wallrs_render::SolidColorRenderer::new(color));
                    let _ = out.set_renderer(
                        solid,
                        &gpu,
                        &self.qh,
                        self.compositor_state.wl_compositor(),
                    );
                    out.current_wallpaper = None;
                    out.audio_track = None;
                    out.audio_handle = None;
                }
            }
        }
    }
}

/// Main engine runner orchestrating Wayland, Calloop, WGPU, and IPC socket server.
pub struct Engine {
    pub conn: Connection,
    pub event_loop: calloop::EventLoop<'static, EngineState>,
    pub state: EngineState,
    pub socket_path: PathBuf,
}

impl Engine {
    /// Creates an engine with the default socket path and standard configuration.
    pub fn new<F>(renderer_factory: F) -> Result<Self, EngineError>
    where
        F: Fn() -> Box<dyn WallpaperRenderer> + 'static,
    {
        Self::with_config(renderer_factory, crate::EngineConfig::default())
    }

    /// Creates an engine with a custom IPC socket path.
    pub fn with_socket<F>(renderer_factory: F, socket_path: PathBuf) -> Result<Self, EngineError>
    where
        F: Fn() -> Box<dyn WallpaperRenderer> + 'static,
    {
        Self::with_config(
            renderer_factory,
            crate::EngineConfig {
                socket_path: Some(socket_path),
                ..Default::default()
            },
        )
    }

    /// Creates an engine with full custom configuration.
    pub fn with_config<F>(
        renderer_factory: F,
        config: crate::EngineConfig,
    ) -> Result<Self, EngineError>
    where
        F: Fn() -> Box<dyn WallpaperRenderer> + 'static,
    {
        let socket_path = config
            .socket_path
            .unwrap_or_else(wallrs_proto::default_socket_path);
        let max_fps = config.max_fps;
        let fullscreen_pause = config.fullscreen_pause;
        let pause_on_maximized = config.pause_on_maximized;
        let allow_audio = config.allow_audio;
        let state_path = config
            .state_path
            .unwrap_or_else(crate::state::default_state_path);
        let restore_state = config.restore_state;
        let state_snapshot = if restore_state {
            crate::state::load_state(&state_path).unwrap_or_default()
        } else {
            crate::state::StateSnapshot::default()
        };

        let layer = config.layer.resolve();
        tracing::info!(layer = ?layer, "Using Wayland layer-shell surface layer");

        tracing::info!("Connecting to Wayland display");
        let conn = Connection::connect_to_env()
            .map_err(|e| EngineError::WaylandConnection(e.to_string()))?;

        let (globals, event_queue) = registry_queue_init(&conn)
            .map_err(|e| EngineError::Global(format!("Registry init failed: {e}")))?;
        let qh = event_queue.handle();

        let compositor_state = CompositorState::bind(&globals, &qh)
            .map_err(|e| EngineError::Global(format!("wl_compositor bind failed: {e}")))?;

        let layer_shell = match LayerShell::bind(&globals, &qh) {
            Ok(ls) => ls,
            Err(wayland_client::globals::BindError::NotPresent) => {
                return Err(EngineError::LayerShellNotSupported);
            }
            Err(e) => {
                return Err(EngineError::Global(format!(
                    "zwlr_layer_shell_v1 bind failed: {e}"
                )));
            }
        };

        let output_state = OutputState::new(&globals, &qh);
        let registry_state = RegistryState::new(&globals);

        let toplevel_manager = if fullscreen_pause {
            match registry_state.bind_one::<ZwlrForeignToplevelManagerV1, EngineState, ()>(
                &qh,
                1..=3,
                (),
            ) {
                Ok(mgr) => {
                    tracing::info!(
                        "zwlr_foreign_toplevel_manager_v1 bound; automatic fullscreen pause active"
                    );
                    Some(mgr)
                }
                Err(e) => {
                    tracing::info!(
                        error = ?e,
                        "zwlr_foreign_toplevel_manager_v1 not available; running without fullscreen pause"
                    );
                    None
                }
            }
        } else {
            None
        };

        let cursor_shape_mgr = registry_state
            .bind_one::<WpCursorShapeManagerV1, EngineState, ()>(&qh, 1..=1, ())
            .ok();

        let mut seat_state = SeatState::new(&globals, &qh);
        let mut pointers = Vec::new();
        let mut cursor_shape_devices = Vec::new();
        for seat in seat_state.seats() {
            if let Ok(pointer) = seat_state.get_pointer(&qh, &seat) {
                if let Some(mgr) = &cursor_shape_mgr {
                    cursor_shape_devices.push(mgr.get_pointer(&pointer, &qh, ()));
                }
                pointers.push(pointer);
            }
        }

        tracing::info!("Initializing WGPU (Vulkan backend)");
        let wgpu_instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });

        let wgpu_adapter =
            pollster::block_on(wgpu_instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            }))
            .map_err(|_| EngineError::AdapterNotFound)?;

        tracing::info!(
            adapter = wgpu_adapter.get_info().name,
            backend = ?wgpu_adapter.get_info().backend,
            "WGPU Adapter acquired"
        );

        let (wgpu_device, wgpu_queue) =
            pollster::block_on(wgpu_adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("wallrs_wgpu_device"),
                ..Default::default()
            }))
            .map_err(|e| EngineError::DeviceRequest(e.to_string()))?;

        let event_loop = calloop::EventLoop::<EngineState>::try_new()
            .map_err(|e| EngineError::EventLoop(e.to_string()))?;

        WaylandSource::new(conn.clone(), event_queue)
            .insert(event_loop.handle())
            .map_err(|e| EngineError::EventLoop(e.to_string()))?;

        // Bind IPC socket and insert into calloop
        let ipc_listener = ipc::bind_socket(&socket_path)?;
        ipc::register_ipc_source(event_loop.handle(), ipc_listener)?;

        let state = EngineState {
            qh,
            registry_state,
            output_state,
            seat_state,
            pointers,
            cursor_shape_mgr,
            cursor_shape_devices,
            compositor_state,
            layer_shell,
            layer,
            wgpu_instance,
            wgpu_adapter,
            wgpu_device,
            wgpu_queue,
            outputs: HashMap::new(),
            renderer_factory: Arc::new(renderer_factory),
            audio_handle: None,
            audio_capture: None,
            max_fps,
            fullscreen_pause,
            pause_on_maximized,
            allow_audio,
            toplevel_manager,
            toplevels: HashMap::new(),
            exit: false,
            state_snapshot,
            state_path,
            restore_state,
        };

        Ok(Self {
            conn,
            event_loop,
            state,
            socket_path,
        })
    }

    /// Runs the main event loop until exit is requested.
    pub fn run(&mut self) -> Result<(), EngineError> {
        tracing::info!("Entering main event loop");
        while !self.state.exit {
            self.event_loop
                .dispatch(None, &mut self.state)
                .map_err(|e| EngineError::EventLoop(e.to_string()))?;
        }
        tracing::info!("Exited main event loop");
        Ok(())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
            tracing::info!(socket = ?self.socket_path, "Cleaned up IPC socket");
        }
    }
}
