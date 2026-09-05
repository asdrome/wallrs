use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, RegistrationToken};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use thiserror::Error;
use wallrs_proto::{Command, OutputInfoProto, OutputSelector, Response};

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
                    paused: out.paused,
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
                    if let Err(e) = out.set_property(&key, value.clone()) {
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

                            if let Err(e) = out.set_renderer(renderer, &gpu) {
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
                    Response::Error("Shader wallpapers will be supported in Phase 4".into())
                }
                "video" => Response::Error("Video wallpapers will be supported in Phase 6".into()),
                other => Response::Error(format!("Unsupported wallpaper type: {other}")),
            }
        }

        Command::Screenshot { .. } => {
            Response::Error("Screenshot will be supported in Phase 7".into())
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
}
