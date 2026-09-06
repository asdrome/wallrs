use clap::{Args, Parser, Subcommand};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use wallrs_proto::{
    Command, OutputInfoProto, OutputSelector, PropertyValue, Response, default_socket_path,
};

#[derive(Parser, Debug)]
#[command(
    name = "wallctl",
    author,
    version,
    about = "Control and query the wallrs live wallpaper daemon",
    long_about = "wallctl communicates with wallrsd over a Unix domain socket to manage active outputs, switch wallpapers, set runtime properties, pause/resume, or terminate the daemon."
)]
struct Cli {
    /// Path to the wallrsd control Unix domain socket
    #[arg(short, long, global = true, value_name = "SOCKET")]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Subcommands,
}

#[derive(Subcommand, Debug)]
enum Subcommands {
    /// List all connected display outputs and their current status
    #[command(alias = "list")]
    ListOutputs(ListOutputsArgs),

    /// Set solid background color on an output or all outputs
    SetColor(SetColorArgs),

    /// Set a dynamic runtime property on an output
    SetProperty(SetPropertyArgs),

    /// Pause wallpaper rendering (saves GPU/CPU when windows are full screen or idle)
    Pause(TargetOutputArgs),

    /// Resume wallpaper rendering
    Resume(TargetOutputArgs),

    /// Toggle wallpaper rendering pause/resume state
    #[command(alias = "toggle")]
    TogglePause(TargetOutputArgs),

    /// Load and display a wallpaper from a manifest folder or wallpaper.toml
    SetWallpaper(SetWallpaperArgs),

    /// Take a screenshot of the current wallpaper on an output and save it to an image file
    Screenshot(ScreenshotArgs),

    /// Gracefully terminate the wallrsd daemon
    Kill,
}

#[derive(Args, Debug)]
struct ScreenshotArgs {
    /// Target output name (e.g. "eDP-1")
    output: String,

    /// Destination file path (e.g. "screenshot.png")
    path: PathBuf,
}

#[derive(Args, Debug)]
struct SetWallpaperArgs {
    /// Path to wallpaper.toml or a directory containing wallpaper.toml
    path: PathBuf,

    /// Target output name (e.g. "eDP-1"). If omitted, applies to all outputs.
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(Args, Debug)]
struct ListOutputsArgs {
    /// Output the list in JSON format
    #[arg(short, long)]
    json: bool,
}

#[derive(Args, Debug)]
struct SetColorArgs {
    /// Color in hex format (e.g., "#0f172a", "ff5500", "#1e1e2eff")
    color: String,

    /// Target output name (e.g. "eDP-1"). If omitted, applies to all outputs.
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(Args, Debug)]
struct SetPropertyArgs {
    /// Property name/key
    key: String,

    /// Property value (number, bool, hex color, or text)
    value: String,

    /// Target output name (e.g. "eDP-1"). If omitted, applies to all outputs.
    #[arg(short, long)]
    output: Option<String>,
}

#[derive(Args, Debug)]
struct TargetOutputArgs {
    /// Target output name (e.g. "eDP-1"). If omitted, applies to all outputs.
    #[arg(short, long)]
    output: Option<String>,
}

/// Parses a hex color string (3, 4, 6, or 8 hex digits, optional leading '#') into RGBA f32 [0.0..1.0].
pub fn parse_hex_color(s: &str) -> Result<[f32; 4], String> {
    let clean = s.trim().trim_start_matches('#');
    let (r, g, b, a) = match clean.len() {
        3 => {
            let r = u8::from_str_radix(&clean[0..1], 16).map_err(|e| e.to_string())? * 17;
            let g = u8::from_str_radix(&clean[1..2], 16).map_err(|e| e.to_string())? * 17;
            let b = u8::from_str_radix(&clean[2..3], 16).map_err(|e| e.to_string())? * 17;
            (r, g, b, 255)
        }
        4 => {
            let r = u8::from_str_radix(&clean[0..1], 16).map_err(|e| e.to_string())? * 17;
            let g = u8::from_str_radix(&clean[1..2], 16).map_err(|e| e.to_string())? * 17;
            let b = u8::from_str_radix(&clean[2..3], 16).map_err(|e| e.to_string())? * 17;
            let a = u8::from_str_radix(&clean[3..4], 16).map_err(|e| e.to_string())? * 17;
            (r, g, b, a)
        }
        6 => {
            let r = u8::from_str_radix(&clean[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&clean[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&clean[4..6], 16).map_err(|e| e.to_string())?;
            (r, g, b, 255)
        }
        8 => {
            let r = u8::from_str_radix(&clean[0..2], 16).map_err(|e| e.to_string())?;
            let g = u8::from_str_radix(&clean[2..4], 16).map_err(|e| e.to_string())?;
            let b = u8::from_str_radix(&clean[4..6], 16).map_err(|e| e.to_string())?;
            let a = u8::from_str_radix(&clean[6..8], 16).map_err(|e| e.to_string())?;
            (r, g, b, a)
        }
        _ => {
            return Err(format!(
                "Invalid hex color '{s}': expected 3, 4, 6, or 8 hex digits"
            ));
        }
    };

    Ok([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ])
}

/// Infers the `PropertyValue` type from a string representation.
pub fn parse_property_value(val: &str) -> PropertyValue {
    if val.eq_ignore_ascii_case("true") {
        return PropertyValue::Bool(true);
    }
    if val.eq_ignore_ascii_case("false") {
        return PropertyValue::Bool(false);
    }
    if let Ok(num) = val.parse::<f32>() {
        return PropertyValue::Number(num);
    }
    if val.starts_with('#')
        && let Ok(color) = parse_hex_color(val)
    {
        return PropertyValue::Color(color);
    }
    PropertyValue::Text(val.to_string())
}

fn print_outputs_table(outputs: &[OutputInfoProto]) {
    if outputs.is_empty() {
        println!("No active outputs detected by wallrsd.");
        return;
    }

    println!("{:<15} {:<15} {:<10}", "OUTPUT", "RESOLUTION", "STATUS");
    println!("{:-<15} {:-<15} {:-<10}", "", "", "");
    for out in outputs {
        let res = format!("{}x{}", out.width, out.height);
        let status = if out.paused { "Paused" } else { "Active" };
        println!("{:<15} {:<15} {:<10}", out.name, res, status);
    }
}

fn send_command(socket_path: &Path, cmd: &Command) -> Result<Response, String> {
    let mut stream = UnixStream::connect(socket_path).map_err(|e| {
        format!(
            "Failed to connect to wallrsd at {:?}: {}\nIs the daemon running? (Try running `wallrsd &`)",
            socket_path, e
        )
    })?;

    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(3)));

    let mut payload =
        serde_json::to_string(cmd).map_err(|e| format!("Failed to serialize command: {e}"))?;
    payload.push('\n');

    stream
        .write_all(payload.as_bytes())
        .map_err(|e| format!("Failed to send command to wallrsd: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("Failed to flush socket: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| format!("Failed to read response from wallrsd: {e}"))?;

    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Err("Daemon closed connection without returning a response".to_string());
    }

    serde_json::from_str(trimmed)
        .map_err(|e| format!("Failed to parse response from wallrsd: {e} (raw: {trimmed})"))
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let socket_path = cli.socket.unwrap_or_else(default_socket_path);

    let (cmd, expect_json) = match cli.command {
        Subcommands::ListOutputs(args) => (Command::ListOutputs, args.json),
        Subcommands::SetColor(args) => {
            let color = parse_hex_color(&args.color)?;
            let selector = match args.output {
                Some(name) => OutputSelector::Named(name),
                None => OutputSelector::All,
            };
            (
                Command::SetProperty {
                    output: selector,
                    key: "color".into(),
                    value: PropertyValue::Color(color),
                },
                false,
            )
        }
        Subcommands::SetProperty(args) => {
            let value = parse_property_value(&args.value);
            let selector = match args.output {
                Some(name) => OutputSelector::Named(name),
                None => OutputSelector::All,
            };
            (
                Command::SetProperty {
                    output: selector,
                    key: args.key,
                    value,
                },
                false,
            )
        }
        Subcommands::Pause(args) => (
            Command::Pause {
                output: args.output,
            },
            false,
        ),
        Subcommands::Resume(args) => (
            Command::Resume {
                output: args.output,
            },
            false,
        ),
        Subcommands::TogglePause(args) => (
            Command::TogglePause {
                output: args.output,
            },
            false,
        ),
        Subcommands::SetWallpaper(args) => {
            let manifest_path = if args.path.is_dir() {
                args.path.join("wallpaper.toml")
            } else {
                args.path
            };
            let canonical = manifest_path.canonicalize().map_err(|e| {
                format!("Failed to find wallpaper manifest at {manifest_path:?}: {e}")
            })?;
            let selector = match args.output {
                Some(name) => OutputSelector::Named(name),
                None => OutputSelector::All,
            };
            (
                Command::SetWallpaper {
                    output: selector,
                    manifest_path: canonical,
                },
                false,
            )
        }
        Subcommands::Screenshot(args) => (
            Command::Screenshot {
                output: args.output,
                path: args.path,
            },
            false,
        ),
        Subcommands::Kill => (Command::Kill, false),
    };

    let resp = send_command(&socket_path, &cmd)?;

    match resp {
        Response::Ok => {
            println!("OK");
            Ok(())
        }
        Response::Outputs(outputs) => {
            if expect_json {
                let json = serde_json::to_string_pretty(&outputs)
                    .map_err(|e| format!("Failed to serialize outputs to JSON: {e}"))?;
                println!("{json}");
            } else {
                print_outputs_table(&outputs);
            }
            Ok(())
        }
        Response::Error(err) => Err(format!("Daemon error: {err}")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color_valid() {
        // #RGB
        let c = parse_hex_color("#fff").unwrap();
        assert_eq!(c, [1.0, 1.0, 1.0, 1.0]);

        // #RGBA
        let c = parse_hex_color("#f008").unwrap();
        assert!((c[0] - 1.0).abs() < 1e-3);
        assert!((c[1] - 0.0).abs() < 1e-3);
        assert!((c[2] - 0.0).abs() < 1e-3);
        assert!((c[3] - 0.533).abs() < 1e-2);

        // #RRGGBB
        let c = parse_hex_color("#00ff00").unwrap();
        assert_eq!(c, [0.0, 1.0, 0.0, 1.0]);

        // without '#'
        let c = parse_hex_color("0000ff").unwrap();
        assert_eq!(c, [0.0, 0.0, 1.0, 1.0]);

        // #RRGGBBAA
        let c = parse_hex_color("#00000080").unwrap();
        assert_eq!(c[0], 0.0);
        assert_eq!(c[1], 0.0);
        assert_eq!(c[2], 0.0);
        assert!((c[3] - (128.0 / 255.0)).abs() < 1e-3);
    }

    #[test]
    fn test_parse_hex_color_invalid() {
        assert!(parse_hex_color("#12").is_err());
        assert!(parse_hex_color("#12345").is_err());
        assert!(parse_hex_color("#zzzzzz").is_err());
    }

    #[test]
    fn test_parse_property_value() {
        assert_eq!(parse_property_value("true"), PropertyValue::Bool(true));
        assert_eq!(parse_property_value("FALSE"), PropertyValue::Bool(false));
        assert_eq!(parse_property_value("42.5"), PropertyValue::Number(42.5));
        assert_eq!(
            parse_property_value("#ffffff"),
            PropertyValue::Color([1.0, 1.0, 1.0, 1.0])
        );
        assert_eq!(
            parse_property_value("hello world"),
            PropertyValue::Text("hello world".into())
        );
    }

    #[test]
    fn test_cli_parse_screenshot() {
        let cli = Cli::try_parse_from(["wallctl", "screenshot", "eDP-1", "test.png"]).unwrap();
        match cli.command {
            Subcommands::Screenshot(args) => {
                assert_eq!(args.output, "eDP-1");
                assert_eq!(args.path, PathBuf::from("test.png"));
            }
            _ => panic!("Expected Subcommands::Screenshot"),
        }
    }

    #[test]
    fn test_cli_parse_toggle_pause() {
        let cli = Cli::try_parse_from(["wallctl", "toggle-pause", "--output", "eDP-1"]).unwrap();
        match cli.command {
            Subcommands::TogglePause(args) => {
                assert_eq!(args.output, Some("eDP-1".into()));
            }
            _ => panic!("Expected Subcommands::TogglePause"),
        }

        let cli_alias = Cli::try_parse_from(["wallctl", "toggle"]).unwrap();
        match cli_alias.command {
            Subcommands::TogglePause(args) => {
                assert_eq!(args.output, None);
            }
            _ => panic!("Expected Subcommands::TogglePause"),
        }
    }
}
