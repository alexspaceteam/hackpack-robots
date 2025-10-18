use anyhow::{anyhow, Result};
use serde_json::Value;
use serialport::SerialPort;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, error, info, warn};

use crate::manifest::Function;
use crate::protocol::{decode_response_by_type, CommandEncoder, ResponseDecoder};
use crate::slip::{slip_encode, SlipDecoder};

#[derive(Debug, Clone, PartialEq)]
pub enum RobotState {
    Disconnected,  // No serial device found
    Connecting,    // Found device, trying to connect
    Connected,     // Connected but not identified
    Initializing,  // Getting device ID
    Ready(String), // Ready with device ID
    Error(String), // Error state with description
}

impl RobotState {
    pub fn is_ready(&self) -> bool {
        matches!(self, RobotState::Ready(_))
    }

    pub fn device_id(&self) -> Option<&str> {
        match self {
            RobotState::Ready(id) => Some(id),
            _ => None,
        }
    }

    pub fn error_message(&self) -> String {
        match self {
            RobotState::Disconnected => "Robot not connected - check USB connection".to_string(),
            RobotState::Connecting => "Robot is connecting - please wait".to_string(),
            RobotState::Connected => "Robot connected but not initialized".to_string(),
            RobotState::Initializing => "Robot is initializing - please wait".to_string(),
            RobotState::Ready(_) => "Robot is ready".to_string(),
            RobotState::Error(msg) => format!("Robot error: {}", msg),
        }
    }
}

pub struct ConnectionManager {
    line_path: String,
    baud_rate: u32,
    state: Arc<Mutex<RobotState>>,
    port: Arc<Mutex<Option<Box<dyn SerialPort>>>>,
}

impl ConnectionManager {
    pub fn new(line_path: String, baud_rate: u32) -> Self {
        Self {
            line_path,
            baud_rate,
            state: Arc::new(Mutex::new(RobotState::Disconnected)),
            port: Arc::new(Mutex::new(None)),
        }
    }

    pub fn get_state(&self) -> RobotState {
        self.state.lock().unwrap().clone()
    }

    pub fn check_and_update_connection(&self) -> Result<()> {
        let current_state = self.get_state();

        // Check if serial device exists
        if !Path::new(&self.line_path).exists() {
            if !matches!(current_state, RobotState::Disconnected) {
                warn!("Serial device {} disappeared", self.line_path);
                self.set_state(RobotState::Disconnected);
                *self.port.lock().unwrap() = None;
            }
            return Ok(());
        }

        match current_state {
            RobotState::Disconnected => {
                info!(
                    "Serial device {} found, attempting connection",
                    self.line_path
                );
                self.set_state(RobotState::Connecting);
                self.attempt_connection()?;
            }
            RobotState::Error(_) => {
                // Retry connection on error
                info!("Retrying connection after error");
                self.set_state(RobotState::Connecting);
                self.attempt_connection()?;
            }
            _ => {
                // For other states, verify connection is still valid
                if let Some(port) = self.port.lock().unwrap().as_mut() {
                    // Try a simple write to check if port is still valid
                    if port.write(&[]).is_err() {
                        warn!("Serial port connection lost");
                        self.set_state(RobotState::Disconnected);
                        *self.port.lock().unwrap() = None;
                    }
                }
            }
        }

        Ok(())
    }

    fn attempt_connection(&self) -> Result<()> {
        match serialport::new(&self.line_path, self.baud_rate)
            .timeout(Duration::from_millis(1000))
            .open()
        {
            Ok(port) => {
                info!("Successfully opened serial port {}", self.line_path);
                *self.port.lock().unwrap() = Some(port);
                self.set_state(RobotState::Connected);

                // Start initialization process
                self.initialize_device()?;
            }
            Err(e) => {
                let error_msg = match e.kind() {
                    serialport::ErrorKind::NoDevice => "Device not found".to_string(),
                    serialport::ErrorKind::InvalidInput => "Invalid device path".to_string(),
                    serialport::ErrorKind::Unknown => {
                        if e.to_string().contains("busy") || e.to_string().contains("in use") {
                            "Serial port is busy - close other applications using this port"
                                .to_string()
                        } else {
                            format!("Connection failed: {}", e)
                        }
                    }
                    _ => format!("Serial port error: {}", e),
                };

                error!("Failed to open serial port: {}", error_msg);
                self.set_state(RobotState::Error(error_msg));
                return Err(anyhow!("Failed to connect"));
            }
        }

        Ok(())
    }

    fn initialize_device(&self) -> Result<()> {
        self.set_state(RobotState::Initializing);

        // Wait for Arduino to initialize
        info!("Waiting 3 seconds for Arduino initialization...");
        std::thread::sleep(Duration::from_secs(3));

        match self.get_device_id() {
            Ok(device_id) => {
                info!("Device initialized with ID: {}", device_id);
                self.set_state(RobotState::Ready(device_id));
            }
            Err(e) => {
                let error_msg = format!("Failed to get device ID: {}", e);
                error!("{}", error_msg);
                self.set_state(RobotState::Error(error_msg));
                return Err(e);
            }
        }

        Ok(())
    }

    fn get_device_id(&self) -> Result<String> {
        let mut port_guard = self.port.lock().unwrap();
        let port = port_guard
            .as_mut()
            .ok_or_else(|| anyhow!("No serial port available"))?;

        // Send deviceId command (tag=0)
        self.send_command(&mut **port, 0)?;

        // Read device ID response
        self.read_response(&mut **port)
    }

    pub fn execute_function(&self, func: &Function, arguments: &Value) -> Result<String> {
        let state = self.get_state();

        if !state.is_ready() {
            return Err(anyhow!("Robot not ready: {}", state.error_message()));
        }

        let mut port_guard = self.port.lock().unwrap();
        let port = port_guard
            .as_mut()
            .ok_or_else(|| anyhow!("No serial port available"))?;

        // Encode and send command
        if func.params.is_empty() {
            self.send_command(&mut **port, func.tag)?;
        } else {
            let mut encoder = CommandEncoder::new();

            for param in &func.params {
                let arg_value = &arguments[&param.name];

                match param.param_type.as_str() {
                    "i16" => {
                        let value = arg_value.as_i64().unwrap() as i16;
                        debug!("Encoding i16 parameter '{}': {}", param.name, value);
                        encoder.write_i16(value);
                    }
                    "i32" => {
                        let value = arg_value.as_i64().unwrap() as i32;
                        debug!("Encoding i32 parameter '{}': {}", param.name, value);
                        encoder.write_i32(value);
                    }
                    "CStr" => {
                        let value = arg_value.as_str().unwrap();
                        debug!("Encoding CStr parameter '{}': '{}'", param.name, value);
                        encoder.write_cstring(value);
                    }
                    _ => {
                        let value = arg_value.as_str().unwrap_or("");
                        debug!(
                            "Encoding unknown type '{}' as CStr: '{}'",
                            param.param_type, value
                        );
                        encoder.write_cstring(value);
                    }
                }
            }

            let args_data = encoder.finish();
            self.send_command_with_args(&mut **port, func.tag, &args_data)?;
        }

        // Read and decode response
        let response_data = self.read_response_raw(&mut **port)?;

        let response_text = if let Some(return_type) = &func.return_type {
            decode_response_by_type(&response_data, return_type)?
        } else {
            "Command executed successfully".to_string()
        };

        debug!("Function '{}' returned: '{}'", func.name, response_text);
        Ok(response_text)
    }

    fn set_state(&self, new_state: RobotState) {
        *self.state.lock().unwrap() = new_state;
    }

    fn send_command(&self, port: &mut dyn SerialPort, tag: u8) -> Result<()> {
        self.send_command_with_args(port, tag, &[])
    }

    fn send_command_with_args(
        &self,
        port: &mut dyn SerialPort,
        tag: u8,
        args_data: &[u8],
    ) -> Result<()> {
        debug!(
            "Sending SLIP command with tag: {} and {} arg bytes",
            tag,
            args_data.len()
        );

        let mut command_data = vec![tag];
        command_data.extend_from_slice(args_data);

        let crc = self.crc8(&command_data);
        command_data.push(crc);

        let slip_frame = slip_encode(&command_data);
        port.write_all(&slip_frame)?;
        port.flush()?;
        debug!("SLIP command sent and flushed ({} bytes)", slip_frame.len());
        Ok(())
    }

    fn read_response(&self, port: &mut dyn SerialPort) -> Result<String> {
        let data = self.read_response_raw(port)?;
        let mut decoder = ResponseDecoder::new(&data);
        decoder.read_cstring()
    }

    fn read_response_raw(&self, port: &mut dyn SerialPort) -> Result<Vec<u8>> {
        debug!("Beginning to read SLIP response from serial port");
        let mut buffer = [0; 256];
        let mut decoder = SlipDecoder::new();

        // Read until we get a complete SLIP frame
        loop {
            match port.read(&mut buffer) {
                Ok(bytes_read) if bytes_read > 0 => {
                    debug!("Read {} bytes from serial", bytes_read);

                    // Process each byte through SLIP decoder
                    for &byte in &buffer[..bytes_read] {
                        if let Some(frame) = decoder.process_byte(byte)? {
                            debug!("Received SLIP frame: {} bytes", frame.len());

                            if frame.len() < 1 {
                                return Err(anyhow!("Frame too short"));
                            }

                            if frame.len() == 1 {
                                // Void function - just CRC, no data
                                debug!("Void function response (CRC only)");
                                return Ok(vec![]);
                            }

                            // Strip CRC (last byte) and return raw data
                            let data = frame[..frame.len() - 1].to_vec();
                            return Ok(data);
                        }
                    }
                }
                Ok(_) => continue,
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    debug!("Serial read timeout");
                    continue;
                }
                Err(e) => {
                    let error_msg = format!("Serial read error: {}", e);
                    self.set_state(RobotState::Error(error_msg.clone()));
                    return Err(anyhow!(error_msg));
                }
            }
        }
    }

    fn crc8(&self, data: &[u8]) -> u8 {
        let mut crc: u8 = 0;
        for &byte in data {
            crc ^= byte;
            for _ in 0..8 {
                if crc & 0x80 != 0 {
                    crc = (crc << 1) ^ 0x07;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }
}
