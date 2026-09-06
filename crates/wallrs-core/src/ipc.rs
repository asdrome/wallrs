use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, RegistrationToken};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use thiserror::Error;
use wallrs_proto::{Command, OutputInfoProto, OutputSelector, PropertyValue, Response};

use crate::engine::EngineState;

#[derive(Debug, Error)]
pub enum IpcError {
    #[error("Another wallrsd instance is already running on socket {0:?}")]
    AlreadyRunning(PathBuf),

    #[error("Failed to bind IPC socket at {0:?}: {1}")]
    Bind(PathBuf, std::io::Error),

    #[error("Failed to register socket with calloop: {0}")]
    Calloop(String),
}

/// Binds a Unix domain socket at `path`, cleaning up any stale socket if found.
pub fn bind_socket(path: &Path) -> Result<UnixListener, IpcError> {
    if path.exists() {
        // Test whether another daemon is actively listening
        if UnixStream::connect(path).is_ok() {
            return Err(IpcError::AlreadyRunning(path.to_path_buf()));
        }
        // Connection failed: remove stale socket file
        tracing::warn!(socket = ?path, "Found stale socket file; cleaning up");
        let _ = std::fs::remove_file(path);
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let listener = UnixListener::bind(path).map_err(|e| IpcError::Bind(path.to_path_buf(), e))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| IpcError::Bind(path.to_path_buf(), e))?;

    tracing::info!(socket = ?path, "IPC control socket bound");
    Ok(listener)
}

/// Registers the IPC `UnixListener` with the Calloop event loop.
pub fn register_ipc_source(
    loop_handle: LoopHandle<'static, EngineState>,
    listener: UnixListener,
) -> Result<RegistrationToken, IpcError> {
    let generic = Generic::new(listener, Interest::READ, Mode::Level);

    loop_handle
        .insert_source(generic, |_, listener, state: &mut EngineState| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        handle_client(stream, state);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        break;
                    }
                    Err(e) => {
                        tracing::error!("Error accepting IPC client connection: {e}");
                        break;
                    }
                }
            }
            Ok(calloop::PostAction::Continue)
        })
        .map_err(|e| IpcError::Calloop(e.to_string()))
}

/// Reads a single NDJSON `Command`, executes it against `EngineState`, and writes a NDJSON `Response`.
fn handle_client(mut stream: UnixStream, state: &mut EngineState) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();

    if let Err(e) = reader.read_line(&mut line) {
        tracing::warn!("Failed reading from IPC client: {e}");
        return;
    }

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    let cmd: Command = match serde_json::from_str(trimmed) {
        Ok(c) => c,
        Err(err) => {
            let resp = Response::Error(format!("Invalid command payload: {err}"));
            let _ = send_response(&mut stream, &resp);
            return;
        }
    };

    let resp = execute_command(cmd, state);
    let _ = send_response(&mut stream, &resp);
}

fn execute_command(cmd: Command, state: &mut EngineState) -> Response {
    match cmd {
        Command::ListOutputs => {
            let list: Vec<OutputInfoProto> = state
                .outputs
                .values()
                .map(|out| OutputInfoProto {
                    name: out.name.clone().unwrap_or_else(|| "unknown".into()),
                    width: out.width,
                    height: out.height,
                    paused: out.is_paused(),
                })
                .collect();
            Response::Outputs(list)
        }

        Command::SetProperty { output, key, value } => {
            let mut matched = false;
            let mut error = None;

            for out in state.outputs.values_mut() {
                let matches = match &output {
                    OutputSelector::All => true,
                    OutputSelector::Named(name) => out.name.as_deref() == Some(name.as_str()),
                    OutputSelector::Span(names) => {
                        out.name.as_ref().is_some_and(|n| names.contains(n))
                    }
                };

                if matches {
                    matched = true;
                    if key == "color"
                        && let PropertyValue::Color(c) = value
                    {
                        let gpu = crate::output::GpuContext {
                            instance: &state.wgpu_instance,
                            adapter: &state.wgpu_adapter,
                            device: &state.wgpu_device,
                            queue: &state.wgpu_queue,
                        };
                        let solid = Box::new(wallrs_render::SolidColorRenderer::new(c));
                        if let Err(e) = out.set_renderer(solid, &gpu, &state.qh) {
                            error = Some(e.to_string());
                            break;
                        }
                        continue;
                    }

                    if let Err(e) = out.set_property(&key, value.clone(), &state.qh) {
                        error = Some(e.to_string());
                        break;
                    }
                }
            }

            if !matched {
                Response::Error(format!("No matching output found for selector {output:?}"))
            } else if let Some(err) = error {
                Response::Error(err)
            } else {
                Response::Ok
            }
        }

        Command::Pause { output } => {
            let mut matched = false;
            for out in state.outputs.values_mut() {
                let matches = match &output {
                    None => true,
                    Some(name) => out.name.as_deref() == Some(name.as_str()),
                };
                if matches {
                    matched = true;
                    out.set_paused(true, &state.qh);
                }
            }
            if !matched && output.is_some() {
                Response::Error(format!("No matching output found for {output:?}"))
            } else {
                Response::Ok
            }
        }

        Command::Resume { output } => {
            let mut matched = false;
            for out in state.outputs.values_mut() {
                let matches = match &output {
                    None => true,
                    Some(name) => out.name.as_deref() == Some(name.as_str()),
                };
                if matches {
                    matched = true;
                    out.set_paused(false, &state.qh);
                }
            }
            if !matched && output.is_some() {
                Response::Error(format!("No matching output found for {output:?}"))
            } else {
                Response::Ok
            }
        }

        Command::TogglePause { output } => match output {
            Some(name) => {
                let mut matched = false;
                for out in state.outputs.values_mut() {
                    if out.name.as_deref() == Some(name.as_str()) {
                        matched = true;
                        let new_paused = !out.manual_paused;
                        out.set_paused(new_paused, &state.qh);
                        break;
                    }
                }
                if !matched {
                    Response::Error(format!("No matching output found for '{name}'"))
                } else {
                    Response::Ok
                }
            }
            None => {
                if state.outputs.is_empty() {
                    Response::Ok
                } else {
                    let any_paused = state.outputs.values().any(|o| o.manual_paused);
                    let target_paused = !any_paused;
                    for out in state.outputs.values_mut() {
                        out.set_paused(target_paused, &state.qh);
                    }
                    Response::Ok
                }
            }
        },

        Command::Kill => {
            tracing::info!("Received Kill command via IPC socket. Shutting down daemon.");
            state.exit = true;
            Response::Ok
        }

        Command::SetWallpaper {
            output,
            manifest_path,
        } => {
            if !manifest_path.exists() {
                return Response::Error(format!("Manifest path does not exist: {manifest_path:?}"));
            }

            let manifest = match wallrs_proto::WallpaperManifest::from_file(&manifest_path) {
                Ok(m) => m,
                Err(e) => return Response::Error(format!("Failed to parse manifest: {e}")),
            };

            let base_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));

            match manifest.wallpaper.r#type.as_str() {
                "image" => {
                    let mut matched = false;
                    let mut error = None;

                    let gpu = crate::output::GpuContext {
                        instance: &state.wgpu_instance,
                        adapter: &state.wgpu_adapter,
                        device: &state.wgpu_device,
                        queue: &state.wgpu_queue,
                    };

                    for out in state.outputs.values_mut() {
                        let matches = match &output {
                            OutputSelector::All => true,
                            OutputSelector::Named(name) => {
                                out.name.as_deref() == Some(name.as_str())
                            }
                            OutputSelector::Span(names) => {
                                out.name.as_ref().is_some_and(|n| names.contains(n))
                            }
                        };

                        if matches {
                            matched = true;
                            let renderer = match wallrs_content_image::ImageRenderer::from_manifest(
                                &manifest, base_dir,
                            ) {
                                Ok(r) => Box::new(r),
                                Err(e) => {
                                    error = Some(e.to_string());
                                    break;
                                }
                            };

                            if let Err(e) = out.set_renderer(renderer, &gpu, &state.qh) {
                                error = Some(e.to_string());
                                break;
                            }
                        }
                    }

                    if !matched {
                        Response::Error(format!("No matching output found for selector {output:?}"))
                    } else if let Some(err) = error {
                        Response::Error(err)
                    } else {
                        Response::Ok
                    }
                }
                "shader" => {
                    let mut matched = false;
                    let mut error = None;

                    let gpu = crate::output::GpuContext {
                        instance: &state.wgpu_instance,
                        adapter: &state.wgpu_adapter,
                        device: &state.wgpu_device,
                        queue: &state.wgpu_queue,
                    };

                    for out in state.outputs.values_mut() {
                        let matches = match &output {
                            OutputSelector::All => true,
                            OutputSelector::Named(name) => {
                                out.name.as_deref() == Some(name.as_str())
                            }
                            OutputSelector::Span(names) => {
                                out.name.as_ref().is_some_and(|n| names.contains(n))
                            }
                        };

                        if matches {
                            matched = true;
                            let renderer =
                                match wallrs_content_shader::ShaderRenderer::from_manifest(
                                    &manifest, base_dir,
                                ) {
                                    Ok(r) => Box::new(r),
                                    Err(e) => {
                                        error = Some(e.to_string());
                                        break;
                                    }
                                };

                            if let Err(e) = out.set_renderer(renderer, &gpu, &state.qh) {
                                error = Some(e.to_string());
                                break;
                            }
                        }
                    }

                    if !matched {
                        Response::Error(format!("No matching output found for selector {output:?}"))
                    } else if let Some(err) = error {
                        Response::Error(err)
                    } else {
                        Response::Ok
                    }
                }
                "video" => {
                    let mut matched = false;
                    let mut error = None;

                    let gpu = crate::output::GpuContext {
                        instance: &state.wgpu_instance,
                        adapter: &state.wgpu_adapter,
                        device: &state.wgpu_device,
                        queue: &state.wgpu_queue,
                    };

                    for out in state.outputs.values_mut() {
                        let matches = match &output {
                            OutputSelector::All => true,
                            OutputSelector::Named(name) => {
                                out.name.as_deref() == Some(name.as_str())
                            }
                            OutputSelector::Span(names) => {
                                out.name.as_ref().is_some_and(|n| names.contains(n))
                            }
                        };

                        if matches {
                            matched = true;
                            let renderer = match wallrs_content_video::VideoRenderer::from_manifest(
                                &manifest, base_dir,
                            ) {
                                Ok(r) => Box::new(r),
                                Err(e) => {
                                    error = Some(e.to_string());
                                    break;
                                }
                            };

                            if let Err(e) = out.set_renderer(renderer, &gpu, &state.qh) {
                                error = Some(e.to_string());
                                break;
                            }
                        }
                    }

                    if !matched {
                        Response::Error(format!("No matching output found for selector {output:?}"))
                    } else if let Some(err) = error {
                        Response::Error(err)
                    } else {
                        Response::Ok
                    }
                }
                other => Response::Error(format!("Unsupported wallpaper type: {other}")),
            }
        }

        Command::Screenshot { output, path } => {
            let Some(out) = state
                .outputs
                .values_mut()
                .find(|o| o.name.as_deref() == Some(output.as_str()))
            else {
                return Response::Error(format!("Output not found: {output}"));
            };

            let Some(renderer) = &mut out.renderer else {
                return Response::Error(format!("Output '{output}' has no active renderer"));
            };

            let width = out.width;
            let height = out.height;
            if width == 0 || height == 0 {
                return Response::Error(format!("Output '{output}' has invalid dimensions"));
            }

            let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let bytes_per_pixel = 4u32;
            let unpadded_bytes_per_row = width * bytes_per_pixel;
            let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
            let buffer_size = (padded_bytes_per_row * height) as wgpu::BufferAddress;

            let capture_texture = state.wgpu_device.create_texture(&wgpu::TextureDescriptor {
                label: Some("screenshot_capture_texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });

            let output_buffer = state.wgpu_device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("screenshot_staging_buffer"),
                size: buffer_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let view = capture_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder =
                state
                    .wgpu_device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("screenshot_encoder"),
                    });

            let now = std::time::Instant::now();
            let elapsed = now.duration_since(out.start_time);
            let delta = out
                .last_frame_time
                .map_or(std::time::Duration::from_millis(16), |l| {
                    now.duration_since(l)
                });
            let spectrum_arc = out.audio_handle.as_ref().map(|h| h.latest());
            let spectrum = spectrum_arc.as_deref().map(|v| v.as_slice());

            let ctx = wallrs_render::FrameContext {
                elapsed,
                delta,
                output_size: (width, height),
                pointer: out.cursor_position,
                spectrum,
                device: &state.wgpu_device,
                queue: &state.wgpu_queue,
            };

            renderer.update(&ctx);
            renderer.render(&mut encoder, &view);

            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &capture_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &output_buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded_bytes_per_row),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            state.wgpu_queue.submit(Some(encoder.finish()));

            // Map buffer and read back pixel data
            let buffer_slice = output_buffer.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            buffer_slice.map_async(wgpu::MapMode::Read, move |res| {
                let _ = tx.send(res);
            });

            if let Err(e) = state.wgpu_device.poll(wgpu::PollType::wait_indefinitely()) {
                return Response::Error(format!("Failed waiting for GPU screenshot readback: {e}"));
            }

            match rx.recv() {
                Ok(Ok(())) => {
                    let mapped_range = match buffer_slice.get_mapped_range() {
                        Ok(v) => v,
                        Err(e) => {
                            return Response::Error(format!("Failed to get mapped range: {e}"));
                        }
                    };
                    let mut unpadded_bytes =
                        Vec::with_capacity((width * height * bytes_per_pixel) as usize);

                    for row in mapped_range.chunks(padded_bytes_per_row as usize) {
                        unpadded_bytes
                            .extend_from_slice(&row[..(width * bytes_per_pixel) as usize]);
                    }

                    drop(mapped_range);
                    output_buffer.unmap();

                    // Create parent directory if needed
                    if let Some(parent) = path.parent()
                        && !parent.as_os_str().is_empty()
                        && !parent.exists()
                        && let Err(e) = std::fs::create_dir_all(parent)
                    {
                        return Response::Error(format!(
                            "Failed to create directory {parent:?}: {e}"
                        ));
                    }

                    if let Err(e) = image::save_buffer(
                        &path,
                        &unpadded_bytes,
                        width,
                        height,
                        image::ExtendedColorType::Rgba8,
                    ) {
                        return Response::Error(format!(
                            "Failed to save screenshot to {path:?}: {e}"
                        ));
                    }

                    Response::Ok
                }
                Ok(Err(e)) => Response::Error(format!("Buffer mapping failed: {e}")),
                Err(_) => Response::Error("Channel disconnected while mapping buffer".into()),
            }
        }
    }
}

fn send_response(stream: &mut UnixStream, resp: &Response) -> std::io::Result<()> {
    let mut data = serde_json::to_string(resp)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    data.push('\n');
    stream.write_all(data.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bind_socket_and_stale_cleanup() {
        let temp_dir = std::env::temp_dir();
        let sock_path = temp_dir.join(format!("wallrs_test_{}.sock", std::process::id()));

        // Bind first time
        let listener = bind_socket(&sock_path).expect("failed to bind socket");
        assert!(sock_path.exists());

        // Binding while actively running should return AlreadyRunning error
        let err = bind_socket(&sock_path);
        assert!(matches!(err, Err(IpcError::AlreadyRunning(_))));

        // Drop the listener (simulating crashed daemon, leaving stale socket file)
        drop(listener);
        assert!(sock_path.exists());

        // Next bind should detect it's stale and re-bind successfully
        let listener2 = bind_socket(&sock_path).expect("failed to clean stale socket and rebind");
        drop(listener2);
        let _ = std::fs::remove_file(&sock_path);
    }

    #[test]
    fn test_command_screenshot_roundtrip() {
        let cmd = Command::Screenshot {
            output: "eDP-1".into(),
            path: PathBuf::from("/tmp/shot.png"),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, parsed);
    }
}
