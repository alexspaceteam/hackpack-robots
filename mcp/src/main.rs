use clap::Parser;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use serialport::SerialPort;
use std::time::Duration;
use serde_json::Value;
use tracing::{info, debug, error};
mod slip;
use slip::{SlipDecoder, slip_encode};
use std::sync::{Arc, Mutex};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, Method, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(name = "arduino-mcp-adapter")]
#[command(about = "MCP adapter for serial Arduino devices")]
struct Cli {
    /// Serial line (e.g. /dev/ttyUSB0)
    #[arg(short, long)]
    line: String,
    
    /// JSON manifest directory
    #[arg(short, long)]
    manifest_dir: PathBuf,
    
    /// HTTP port for MCP server
    #[arg(short, long, default_value = "8080")]
    port: u16,
    
    /// Baud rate
    #[arg(short, long, default_value = "115200")]
    baud: u32,
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

#[derive(Debug, Serialize, Deserialize)]
struct McpRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct McpResponse {
    jsonrpc: String,
    id: Option<Value>,
    result: Option<Value>,
    error: Option<McpError>,
}

#[derive(Debug, Serialize, Deserialize)]
struct McpError {
    code: i32,
    message: String,
    data: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Tool {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    let cli = Cli::parse();
    
    // Open serial port
    let mut port = serialport::new(&cli.line, cli.baud)
        .timeout(Duration::from_millis(1000))
        .open()?;
    
    info!("Connected to {}", cli.line);
    
    // Wait for Arduino to initialize
    info!("Waiting 3 seconds for Arduino initialization...");
    std::thread::sleep(Duration::from_secs(3));
    
    // Send deviceId command (tag=0)
    send_command(&mut *port, 0)?;
    
    // Read device ID response
    let device_id = read_response(&mut *port)?;
    info!("Device ID: {}", device_id);
    
    // Load manifest based on device ID
    let manifest_path = cli.manifest_dir.join(format!("{}.json", device_id));
    let manifest = load_manifest(&manifest_path)?;
    
    info!("Loaded manifest for {}: {}", manifest.name, manifest.description);
    info!("Available functions: {}", manifest.functions.len());
    
    // Print function list
    for func in &manifest.functions {
        let params_str = if func.params.is_empty() {
            "()".to_string()
        } else {
            let params: Vec<String> = func.params.iter()
                .map(|p| format!("{}: {}", p.name, p.param_type))
                .collect();
            format!("({})", params.join(", "))
        };
        let return_str = func.return_type.as_ref()
            .map(|t| format!(" -> {}", t))
            .unwrap_or_default();
        info!("  {}. {}{}{} - {}", func.tag, func.name, params_str, return_str, func.desc);
    }
    
    // Start HTTP MCP server
    let serial_port = Arc::new(Mutex::new(port));
    let manifest = Arc::new(manifest);
    start_http_server(cli.port, serial_port, manifest).await?;
    
    Ok(())
}

fn send_command_with_args(port: &mut dyn SerialPort, tag: u8, args_data: &[u8]) -> Result<()> {
    debug!("Sending SLIP command with tag: {} and {} arg bytes", tag, args_data.len());
    
    let mut command_data = vec![tag];
    command_data.extend_from_slice(args_data);
    
    let crc = crc8(&command_data);
    command_data.push(crc);
    
    let slip_frame = slip_encode(&command_data);
    port.write_all(&slip_frame)?;
    port.flush()?;
    debug!("SLIP command sent and flushed ({} bytes)", slip_frame.len());
    Ok(())
}

fn send_command(port: &mut dyn SerialPort, tag: u8) -> Result<()> {
    send_command_with_args(port, tag, &[])
}

fn crc8(data: &[u8]) -> u8 {
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

struct ResponseDecoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ResponseDecoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    
    fn read_i16(&mut self) -> Result<i16> {
        if self.pos + 2 > self.data.len() {
            return Err(anyhow!("Not enough data for i16"));
        }
        let value = i16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]);
        self.pos += 2;
        Ok(value)
    }
    
    fn read_i32(&mut self) -> Result<i32> {
        if self.pos + 4 > self.data.len() {
            return Err(anyhow!("Not enough data for i32"));
        }
        let value = i32::from_le_bytes([
            self.data[self.pos], 
            self.data[self.pos + 1], 
            self.data[self.pos + 2], 
            self.data[self.pos + 3]
        ]);
        self.pos += 4;
        Ok(value)
    }
    
    fn read_cstring(&mut self) -> Result<String> {
        let remaining = &self.data[self.pos..];
        
        // Find null terminator or use all remaining data
        let end_pos = remaining.iter().position(|&b| b == 0).unwrap_or(remaining.len());
        
        if end_pos == 0 && remaining.len() > 0 && remaining[0] == 0 {
            // Empty string with null terminator
            self.pos += 1;
            return Ok(String::new());
        }
        
        let str_bytes = &remaining[..end_pos];
        let result = String::from_utf8_lossy(str_bytes).to_string();
        
        // Skip past the string and null terminator if present
        self.pos += end_pos;
        if self.pos < self.data.len() && self.data[self.pos] == 0 {
            self.pos += 1; // Skip null terminator
        }
        
        debug!("Decoded C string: '{}'", result);
        Ok(result)
    }
}

struct CommandEncoder {
    data: Vec<u8>,
}

impl CommandEncoder {
    fn new() -> Self {
        Self { data: Vec::new() }
    }
    
    fn write_i16(&mut self, value: i16) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }
    
    fn write_i32(&mut self, value: i32) {
        self.data.extend_from_slice(&value.to_le_bytes());
    }
    
    fn write_cstring(&mut self, value: &str) {
        self.data.extend_from_slice(value.as_bytes());
        self.data.push(0); // Null terminator
    }
    
    fn finish(self) -> Vec<u8> {
        self.data
    }
}

fn read_response_raw(port: &mut dyn SerialPort) -> Result<Vec<u8>> {
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
                        
                        // Handle void functions (frame with just CRC)
                        if frame.len() < 1 {
                            return Err(anyhow!("Frame too short"));
                        }
                        
                        if frame.len() == 1 {
                            // Void function - just CRC, no data
                            debug!("Void function response (CRC only)");
                            return Ok(vec![]);
                        }
                        
                        // Strip CRC (last byte) and return raw data
                        let data = frame[..frame.len()-1].to_vec(); // Remove CRC
                        return Ok(data);
                    }
                }
            },
            Ok(_) => continue,
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                debug!("Serial read timeout");
                continue;
            },
            Err(e) => return Err(anyhow!("Serial read error: {}", e)),
        }
    }
}

fn read_response(port: &mut dyn SerialPort) -> Result<String> {
    let data = read_response_raw(port)?;
    let mut decoder = ResponseDecoder::new(&data);
    decoder.read_cstring()
}

fn decode_response_by_type(data: &[u8], return_type: &str) -> Result<String> {
    // Handle void functions (no data)
    if data.is_empty() {
        return Ok("Command executed successfully".to_string());
    }
    
    let mut decoder = ResponseDecoder::new(data);
    
    match return_type {
        "CStr" => decoder.read_cstring(),
        "i16" => {
            let value = decoder.read_i16()?;
            Ok(value.to_string())
        },
        "i32" => {
            let value = decoder.read_i32()?;
            Ok(value.to_string())
        },
        _ => decoder.read_cstring(), // Default to string
    }
}

fn load_manifest(path: &PathBuf) -> Result<Manifest> {
    let content = std::fs::read_to_string(path)?;
    let manifest: Manifest = serde_json::from_str(&content)?;
    Ok(manifest)
}

type SerialPortType = Arc<Mutex<Box<dyn SerialPort>>>;
type ManifestType = Arc<Manifest>;

async fn start_http_server(port: u16, serial_port: SerialPortType, manifest: ManifestType) -> Result<()> {
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    info!("MCP HTTP server listening on {}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let serial_port = Arc::clone(&serial_port);
        let manifest = Arc::clone(&manifest);

        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(stream);
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(move |req| {
                    handle_mcp_request(req, Arc::clone(&serial_port), Arc::clone(&manifest))
                }))
                .await
            {
                error!("Connection error: {}", err);
            }
        });
    }
}

async fn handle_mcp_request(
    req: Request<hyper::body::Incoming>,
    serial_port: SerialPortType,
    manifest: ManifestType,
) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
    
    let response = match req.method() {
        &Method::POST => {
            match req.uri().path() {
                "/mcp" => handle_mcp_post(req, serial_port, manifest).await,
                _ => Ok(not_found_response()),
            }
        }
        &Method::OPTIONS => {
            Ok(cors_response())
        }
        _ => Ok(not_found_response()),
    };
    
    response
}

async fn handle_mcp_post(
    req: Request<hyper::body::Incoming>,
    serial_port: SerialPortType,
    manifest: ManifestType,
) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
    
    let body_bytes = req.collect().await?.to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    
    debug!("Received MCP request: {}", body_str);
    
    let request: McpRequest = match serde_json::from_str(&body_str) {
        Ok(req) => req,
        Err(e) => {
            error!("Failed to parse MCP request: {}", e);
            let detailed_error = format!("JSON parse error: {}. Check your JSON syntax - you may have missing quotes, extra commas, or malformed structure.", e);
            return Ok(error_response(-32700, &detailed_error));
        }
    };
    
    let response = match request.method.as_str() {
        "initialize" => handle_initialize(&request).await,
        "tools/list" => handle_tools_list(&request, &manifest).await,
        "tools/call" => handle_tools_call(&request, &serial_port, &manifest).await,
        _ => {
            McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(McpError {
                    code: -32601,
                    message: "Method not found".to_string(),
                    data: None,
                }),
            }
        }
    };
    
    let response_json = serde_json::to_string(&response).unwrap();
    debug!("Sending MCP response: {}", response_json);
    
    Ok(json_response(response_json))
}

async fn handle_initialize(_request: &McpRequest) -> McpResponse {
    let result = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "arduino-mcp-adapter",
            "version": "0.1.0"
        }
    });
    
    McpResponse {
        jsonrpc: "2.0".to_string(),
        id: _request.id.clone(),
        result: Some(result),
        error: None,
    }
}

async fn handle_tools_list(_request: &McpRequest, manifest: &Manifest) -> McpResponse {
    let tools: Vec<Tool> = manifest.functions.iter().map(|func| {
        Tool {
            name: func.name.clone(),
            description: func.desc.clone(),
            input_schema: create_input_schema(func),
        }
    }).collect();
    
    let result = serde_json::json!({
        "tools": tools
    });
    
    McpResponse {
        jsonrpc: "2.0".to_string(),
        id: _request.id.clone(),
        result: Some(result),
        error: None,
    }
}

async fn handle_tools_call(
    request: &McpRequest,
    serial_port: &SerialPortType,
    manifest: &Manifest,
) -> McpResponse {
    
    let params = match request.params.as_ref() {
        Some(p) => p,
        None => {
            return McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: None,
                error: Some(McpError {
                    code: -32602,
                    message: "Missing params".to_string(),
                    data: None,
                }),
            };
        }
    };
    
    let tool_name = match params["name"].as_str() {
        Some(name) => name,
        None => {
            return McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: None,
                error: Some(McpError {
                    code: -32602,
                    message: "Missing tool name".to_string(),
                    data: None,
                }),
            };
        }
    };
    
    let empty_args = serde_json::json!({});
    let arguments = params.get("arguments").unwrap_or(&empty_args);
    
    // Find the function in manifest
    let func = match manifest.functions.iter().find(|f| f.name == tool_name) {
        Some(f) => f,
        None => {
            return McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: None,
                error: Some(McpError {
                    code: -32602,
                    message: format!("Function not found: {}", tool_name),
                    data: None,
                }),
            };
        }
    };
    
    // Execute the function via serial
    match execute_arduino_function(serial_port, func, arguments).await {
        Ok(response_text) => {
            let result = serde_json::json!({
                "content": [
                    {
                        "type": "text",
                        "text": response_text
                    }
                ]
            });
            
            McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: Some(result),
                error: None,
            }
        }
        Err(e) => {
            McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: None,
                error: Some(McpError {
                    code: -32603,
                    message: format!("Execution error: {}", e),
                    data: None,
                }),
            }
        }
    }
}

fn type_to_json_type(rust_type: &str) -> &'static str {
    match rust_type {
        "i16" | "i32" | "i64" => "integer",
        "f32" | "f64" => "number",
        "CStr" => "string",
        "bool" => "boolean",
        _ => "string", // Default fallback
    }
}

fn validate_function_arguments(func: &Function, arguments: &Value) -> Result<()> {
    let args_obj = arguments.as_object().ok_or_else(|| anyhow!("Arguments must be an object"))?;
    
    // Check if function expects no parameters but arguments were provided
    if func.params.is_empty() && !args_obj.is_empty() {
        let provided_params: Vec<String> = args_obj.keys().cloned().collect();
        return Err(anyhow!(
            "Function '{}' takes no parameters, but you provided: [{}]. Remove all arguments.",
            func.name, 
            provided_params.join(", ")
        ));
    }
    
    // Check if function expects parameters but none were provided
    if !func.params.is_empty() && args_obj.is_empty() {
        let param_specs: Vec<String> = func.params.iter()
            .map(|p| format!("{}: {}", p.name, type_to_json_type(&p.param_type)))
            .collect();
        return Err(anyhow!(
            "Function '{}' requires {} parameters: [{}]. Please provide all required arguments.",
            func.name,
            func.params.len(),
            param_specs.join(", ")
        ));
    }
    
    // Check for unexpected parameters first (more actionable error)
    for arg_name in args_obj.keys() {
        if !func.params.iter().any(|p| &p.name == arg_name) {
            let param_specs: Vec<String> = func.params.iter()
                .map(|p| format!("{}: {}", p.name, type_to_json_type(&p.param_type)))
                .collect();
            return Err(anyhow!(
                "Invalid parameter '{}' for function '{}'. Valid parameters are: [{}]. Please correct the parameter name.",
                arg_name, 
                func.name,
                param_specs.join(", ")
            ));
        }
    }
    
    // Check each required parameter
    for param in &func.params {
        if !args_obj.contains_key(&param.name) {
            return Err(anyhow!(
                "Missing required parameter '{}' (type: {}) for function '{}'. Please add this parameter to your arguments.",
                param.name,
                type_to_json_type(&param.param_type),
                func.name
            ));
        }
        
        let arg_value = &arguments[&param.name];
        
        // Validate parameter type
        match param.param_type.as_str() {
            "i16" | "i32" => {
                if !arg_value.is_number() {
                    return Err(anyhow!(
                        "Parameter '{}' must be a number (type: {}), but got {}. Please provide a numeric value.",
                        param.name,
                        type_to_json_type(&param.param_type),
                        arg_value
                    ));
                }
                
                if param.param_type == "i16" {
                    let value = arg_value.as_i64().unwrap_or(0);
                    if value < i16::MIN as i64 || value > i16::MAX as i64 {
                        return Err(anyhow!(
                            "Parameter '{}' value {} is out of range for i16 ({} to {}). Please use a value within this range.",
                            param.name,
                            value,
                            i16::MIN,
                            i16::MAX
                        ));
                    }
                }
            },
            "CStr" => {
                if !arg_value.is_string() {
                    return Err(anyhow!(
                        "Parameter '{}' must be a string, but got {}. Please provide a string value in quotes.",
                        param.name,
                        arg_value
                    ));
                }
            },
            "bool" => {
                if !arg_value.is_boolean() {
                    return Err(anyhow!(
                        "Parameter '{}' must be a boolean (true/false), but got {}. Please use true or false.",
                        param.name,
                        arg_value
                    ));
                }
            },
            _ => {
                // Unknown types - accept any value and try to convert to string
            }
        }
    }
    
    Ok(())
}

async fn execute_arduino_function(
    serial_port: &SerialPortType,
    func: &Function,
    arguments: &Value,
) -> Result<String> {
    // Validate arguments first
    validate_function_arguments(func, arguments)?;
    
    let mut port = serial_port.lock().unwrap();
    
    // Encode arguments if any
    if func.params.is_empty() {
        // No parameters - just send the tag
        send_command(&mut **port, func.tag)?;
    } else {
        // Encode parameters based on manifest
        let mut encoder = CommandEncoder::new();
        
        for param in &func.params {
            let arg_value = &arguments[&param.name];
            
            match param.param_type.as_str() {
                "i16" => {
                    let value = arg_value.as_i64().unwrap() as i16; // Safe after validation
                    debug!("Encoding i16 parameter '{}': {}", param.name, value);
                    encoder.write_i16(value);
                },
                "i32" => {
                    let value = arg_value.as_i64().unwrap() as i32; // Safe after validation
                    debug!("Encoding i32 parameter '{}': {}", param.name, value);
                    encoder.write_i32(value);
                },
                "CStr" => {
                    let value = arg_value.as_str().unwrap(); // Safe after validation
                    debug!("Encoding CStr parameter '{}': '{}'", param.name, value);
                    encoder.write_cstring(value);
                },
                _ => {
                    let value = arg_value.as_str().unwrap_or("");
                    debug!("Encoding unknown type '{}' as CStr: '{}'", param.param_type, value);
                    encoder.write_cstring(value);
                }
            }
        }
        
        let args_data = encoder.finish();
        send_command_with_args(&mut **port, func.tag, &args_data)?;
    }
    
    // Read and decode response
    let response_data = read_response_raw(&mut **port)?;
    
    let response_text = if let Some(return_type) = &func.return_type {
        decode_response_by_type(&response_data, return_type)?
    } else {
        "Command executed successfully".to_string()
    };
    
    debug!("Function '{}' returned: '{}'", func.name, response_text);
    Ok(response_text)
}

fn json_response(body: String) -> Response<BoxBody<hyper::body::Bytes, hyper::Error>> {
    Response::builder()
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header("Access-Control-Allow-Headers", "Content-Type")
        .body(BoxBody::new(Full::new(body.into()).map_err(|e| match e {})))
        .unwrap()
}

fn cors_response() -> Response<BoxBody<hyper::body::Bytes, hyper::Error>> {
    Response::builder()
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header("Access-Control-Allow-Headers", "Content-Type")
        .body(BoxBody::new(Full::new("".into()).map_err(|e| match e {})))
        .unwrap()
}

fn not_found_response() -> Response<BoxBody<hyper::body::Bytes, hyper::Error>> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(BoxBody::new(Full::new("Not Found".into()).map_err(|e| match e {})))
        .unwrap()
}

fn error_response(code: i32, message: &str) -> Response<BoxBody<hyper::body::Bytes, hyper::Error>> {
    let error = McpResponse {
        jsonrpc: "2.0".to_string(),
        id: None,
        result: None,
        error: Some(McpError {
            code,
            message: message.to_string(),
            data: None,
        }),
    };
    
    let body = serde_json::to_string(&error).unwrap();
    json_response(body)
}

fn create_input_schema(func: &Function) -> Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    
    for param in &func.params {
        let param_schema = match param.param_type.as_str() {
            "i16" | "i32" | "i64" => serde_json::json!({"type": "integer"}),
            "f32" | "f64" => serde_json::json!({"type": "number"}),
            "CStr" => serde_json::json!({"type": "string"}),
            "bool" => serde_json::json!({"type": "boolean"}),
            _ => serde_json::json!({"type": "string"}),
        };
        properties.insert(param.name.clone(), param_schema);
        required.push(param.name.clone());
    }
    
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required
    })
}