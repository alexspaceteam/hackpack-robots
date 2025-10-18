use anyhow::{anyhow, Context, Result};
use clap::Parser;
use nix::fcntl::OFlag;
use nix::pty::{grantpt, posix_openpt, ptsname, unlockpt, PtyMaster};
use nix::unistd::read;
use serde::{Deserialize, Serialize};
use std::fs;
use std::os::unix::fs as unix_fs;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

// Re-use SLIP protocol constants and logic
mod protocol;
mod slip;

use protocol::{crc8, decode_command, encode_response, ResponseData};
use slip::{slip_encode, SlipDecoder};

#[derive(Parser, Debug)]
#[command(name = "arduino-simulator")]
#[command(about = "Arduino simulator for testing MCP communication")]
#[command(
    long_about = "Simulates an Arduino device by creating a PTY and implementing the MCP serial protocol"
)]
struct Args {
    #[arg(short, long, help = "Path to symlink for the PTY (e.g., /tmp/mytty)")]
    line: PathBuf,

    #[arg(short, long, help = "Path to JSON manifest file")]
    manifest: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct Manifest {
    name: String,
    description: String,
    version: String,
    functions: Vec<Function>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Function {
    tag: u8,
    name: String,
    desc: String,
    #[serde(rename = "return")]
    return_type: Option<String>,
    params: Vec<Parameter>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Parameter {
    name: String,
    #[serde(rename = "type")]
    param_type: String,
}

struct PtySymlink {
    symlink_path: PathBuf,
}

impl PtySymlink {
    fn new(symlink_path: PathBuf, target_path: &Path) -> Result<Self> {
        // Remove existing symlink if it exists
        if symlink_path.exists() {
            info!("Removing existing symlink at {}", symlink_path.display());
            fs::remove_file(&symlink_path).with_context(|| {
                format!(
                    "Failed to remove existing symlink: {}",
                    symlink_path.display()
                )
            })?;
        }

        // Create new symlink
        info!(
            "Creating symlink {} -> {}",
            symlink_path.display(),
            target_path.display()
        );
        unix_fs::symlink(target_path, &symlink_path)
            .with_context(|| format!("Failed to create symlink: {}", symlink_path.display()))?;

        Ok(Self { symlink_path })
    }
}

impl Drop for PtySymlink {
    fn drop(&mut self) {
        if self.symlink_path.exists() {
            info!("Cleaning up symlink at {}", self.symlink_path.display());
            if let Err(e) = fs::remove_file(&self.symlink_path) {
                error!("Failed to remove symlink: {}", e);
            }
        }
    }
}

struct Simulator {
    manifest: Manifest,
    device_id: String,
    pty_master: PtyMaster,
    _symlink: PtySymlink,
    slip_decoder: SlipDecoder,
}

impl Simulator {
    fn new(args: Args) -> Result<Self> {
        // Load manifest
        let manifest_content = fs::read_to_string(&args.manifest).with_context(|| {
            format!("Failed to read manifest file: {}", args.manifest.display())
        })?;

        let manifest: Manifest = serde_json::from_str(&manifest_content).with_context(|| {
            format!("Failed to parse manifest file: {}", args.manifest.display())
        })?;

        // Derive device ID from manifest filename (without .json extension)
        let device_id = args
            .manifest
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("Invalid manifest filename"))?
            .to_string();

        info!(
            "Loaded manifest: {} ({})",
            manifest.name, manifest.description
        );
        info!("Device ID: {}", device_id);
        info!("Functions defined: {}", manifest.functions.len());

        for func in &manifest.functions {
            let params_str = if func.params.is_empty() {
                "()".to_string()
            } else {
                let params: Vec<String> = func
                    .params
                    .iter()
                    .map(|p| format!("{}: {}", p.name, p.param_type))
                    .collect();
                format!("({})", params.join(", "))
            };
            let return_str = func
                .return_type
                .as_ref()
                .map(|t| format!(" -> {}", t))
                .unwrap_or_default();
            info!(
                "  [{}] {}{}{} - {}",
                func.tag, func.name, params_str, return_str, func.desc
            );
        }

        // Create PTY with non-blocking mode for graceful shutdown
        let pty_master = posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY | OFlag::O_NONBLOCK)
            .context("Failed to create PTY")?;

        grantpt(&pty_master).context("Failed to grant PTY")?;
        unlockpt(&pty_master).context("Failed to unlock PTY")?;

        let slave_name = unsafe { ptsname(&pty_master) }.context("Failed to get PTY slave name")?;

        info!("PTY master created");
        info!("PTY slave: {}", slave_name);

        // Create symlink
        let symlink = PtySymlink::new(args.line.clone(), Path::new(&slave_name))?;
        info!("Symlink created at: {}", args.line.display());

        Ok(Self {
            manifest,
            device_id,
            pty_master,
            _symlink: symlink,
            slip_decoder: SlipDecoder::new(),
        })
    }

    fn handle_command(&self, frame: &[u8]) -> Result<Vec<u8>> {
        // Decode command frame (tag + args + CRC)
        let (tag, args) = decode_command(frame)?;

        debug!(
            "Received command - Tag: {}, Args: {} bytes",
            tag,
            args.len()
        );

        // Handle tag 0 (deviceId) specially
        if tag == 0 {
            info!("[deviceId()] -> \"{}\"", self.device_id);
            let response = encode_response(&ResponseData::CStr(self.device_id.clone()))?;
            return Ok(response);
        }

        // Find function in manifest
        let func = self
            .manifest
            .functions
            .iter()
            .find(|f| f.tag == tag)
            .ok_or_else(|| {
                warn!("Unknown function tag: {}", tag);
                anyhow!("Unknown function tag: {}", tag)
            })?;

        // Parse arguments
        let parsed_args = self.parse_arguments(&func.params, args)?;

        // Log function call
        let args_display = if func.params.is_empty() {
            String::new()
        } else {
            let args_str: Vec<String> = func
                .params
                .iter()
                .zip(parsed_args.iter())
                .map(|(p, v)| format!("{}={}", p.name, v))
                .collect();
            args_str.join(", ")
        };

        // Generate stub response based on return type
        let response_data = match func.return_type.as_deref() {
            None => {
                info!("[{}({})] -> void", func.name, args_display);
                ResponseData::Void
            }
            Some("i16") => {
                info!("[{}({})] -> 0 (i16)", func.name, args_display);
                ResponseData::I16(0)
            }
            Some("i32") => {
                info!("[{}({})] -> 0 (i32)", func.name, args_display);
                ResponseData::I32(0)
            }
            Some("CStr") => {
                info!("[{}({})] -> \"\" (CStr)", func.name, args_display);
                ResponseData::CStr(String::new())
            }
            Some(other) => {
                warn!("Unknown return type: {}, returning empty string", other);
                ResponseData::CStr(String::new())
            }
        };

        let response = encode_response(&response_data)?;
        Ok(response)
    }

    fn parse_arguments(&self, params: &[Parameter], args: &[u8]) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let mut offset = 0;

        for param in params {
            match param.param_type.as_str() {
                "i16" => {
                    if offset + 2 > args.len() {
                        return Err(anyhow!("Not enough data for i16 parameter"));
                    }
                    let value = i16::from_le_bytes([args[offset], args[offset + 1]]);
                    result.push(value.to_string());
                    offset += 2;
                }
                "i32" => {
                    if offset + 4 > args.len() {
                        return Err(anyhow!("Not enough data for i32 parameter"));
                    }
                    let value = i32::from_le_bytes([
                        args[offset],
                        args[offset + 1],
                        args[offset + 2],
                        args[offset + 3],
                    ]);
                    result.push(value.to_string());
                    offset += 4;
                }
                "CStr" => {
                    let end = args[offset..]
                        .iter()
                        .position(|&b| b == 0)
                        .map(|p| offset + p)
                        .unwrap_or(args.len());
                    let s = String::from_utf8_lossy(&args[offset..end]).to_string();
                    result.push(format!("\"{}\"", s));
                    offset = end + 1; // Skip null terminator
                }
                _ => {
                    return Err(anyhow!("Unknown parameter type: {}", param.param_type));
                }
            }
        }

        Ok(result)
    }

    fn send_error_response(&mut self, error_code: u8) -> Result<()> {
        // Error frame: [0xFF] [error_code] [CRC]
        let mut frame = vec![0xFF, error_code];
        let crc = crc8(&frame);
        frame.push(crc);

        let encoded = slip_encode(&frame);
        self.write_to_pty(&encoded)?;

        Ok(())
    }

    fn write_to_pty(&mut self, data: &[u8]) -> Result<()> {
        let fd = self.pty_master.as_raw_fd();
        nix::unistd::write(fd, data).context("Failed to write to PTY")?;
        Ok(())
    }

    fn run(&mut self, running: Arc<AtomicBool>) -> Result<()> {
        info!("Simulator running - waiting for connections...");

        let fd = self.pty_master.as_raw_fd();
        let mut buffer = [0u8; 256];
        let mut connected = false;

        while running.load(Ordering::Relaxed) {
            match read(fd, &mut buffer) {
                Ok(0) => {
                    // EOF - shouldn't normally happen for PTY, but handle it
                    if connected {
                        info!("Client disconnected (EOF)");
                        connected = false;
                        self.slip_decoder.reset();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Ok(n) => {
                    if !connected {
                        info!("Client connected");
                        connected = true;
                        self.slip_decoder.reset();
                    }

                    debug!("Read {} bytes from PTY", n);

                    // Process each byte through SLIP decoder
                    for &byte in &buffer[..n] {
                        match self.slip_decoder.process_byte(byte) {
                            Ok(Some(frame)) => {
                                debug!("SLIP frame complete: {} bytes", frame.len());

                                // Process the command
                                match self.handle_command(&frame) {
                                    Ok(response) => {
                                        let encoded = slip_encode(&response);
                                        debug!("Sending response: {} bytes", encoded.len());
                                        if let Err(e) = self.write_to_pty(&encoded) {
                                            error!("Failed to send response: {}", e);
                                            // Write failure likely means disconnect
                                            if connected {
                                                info!("Client disconnected (write error)");
                                                connected = false;
                                                self.slip_decoder.reset();
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        if e.to_string().contains("Unknown function tag") {
                                            error!("Dispatch error: {}", e);
                                            let _ = self.send_error_response(0x02);
                                        // Dispatch error
                                        } else {
                                            error!("CRC or protocol error: {}", e);
                                            let _ = self.send_error_response(0x01);
                                            // CRC mismatch
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                // Still accumulating frame
                            }
                            Err(e) => {
                                error!("SLIP decode error: {}", e);
                                let _ = self.send_error_response(0x01);
                            }
                        }
                    }
                }
                Err(nix::errno::Errno::EAGAIN) => {
                    // No data available, sleep briefly
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(nix::errno::Errno::EIO) => {
                    // I/O error - typically means client disconnected
                    if connected {
                        info!("Client disconnected (I/O error)");
                        connected = false;
                        self.slip_decoder.reset();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(e) => {
                    // Other errors - log and continue
                    warn!("PTY read error: {}, continuing...", e);
                    if connected {
                        info!("Client disconnected (error: {})", e);
                        connected = false;
                        self.slip_decoder.reset();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }

        info!("Simulator shutting down");
        Ok(())
    }
}

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let args = Args::parse();

    info!("Arduino Simulator starting...");
    info!("Line: {}", args.line.display());
    info!("Manifest: {}", args.manifest.display());

    // Validate arguments
    if !args.manifest.exists() {
        return Err(anyhow!(
            "Manifest file does not exist: {}",
            args.manifest.display()
        ));
    }

    let mut simulator = Simulator::new(args)?;

    // Set up Ctrl+C handler
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        info!("Received Ctrl+C, shutting down...");
        r.store(false, Ordering::Relaxed);
    })
    .context("Failed to set Ctrl+C handler")?;

    // Run simulator
    simulator.run(running)?;

    Ok(())
}
