use anyhow::Result;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{debug, error, info};

use crate::connection::ConnectionManager;
use crate::manifest::{Manifest, ManifestManager, Tool};
use crate::python_runner;

#[derive(Debug, Serialize, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    pub params: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<McpError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

pub struct McpServer {
    connection_manager: Arc<ConnectionManager>,
    manifest_manager: Arc<ManifestManager>,
}

impl McpServer {
    pub fn new(
        connection_manager: Arc<ConnectionManager>,
        manifest_manager: Arc<ManifestManager>,
    ) -> Self {
        Self {
            connection_manager,
            manifest_manager,
        }
    }

    pub async fn start(&self, port: u16) -> Result<()> {
        let addr = format!("0.0.0.0:{}", port);
        let base_url = Arc::new(format!("http://127.0.0.1:{}/mcp", port));
        let listener = TcpListener::bind(&addr).await?;
        info!("MCP HTTP server listening on {}", addr);

        // Start connection monitoring in background
        let connection_manager = Arc::clone(&self.connection_manager);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if let Err(e) = connection_manager.check_and_update_connection() {
                    error!("Connection check error: {}", e);
                }
            }
        });

        loop {
            let (stream, _) = listener.accept().await?;
            let connection_manager = Arc::clone(&self.connection_manager);
            let manifest_manager = Arc::clone(&self.manifest_manager);
            let base_url = Arc::clone(&base_url);

            tokio::spawn(async move {
                let io = hyper_util::rt::TokioIo::new(stream);
                if let Err(err) = http1::Builder::new()
                    .serve_connection(
                        io,
                        service_fn(move |req| {
                            Self::handle_request(
                                req,
                                Arc::clone(&connection_manager),
                                Arc::clone(&manifest_manager),
                                Arc::clone(&base_url),
                            )
                        }),
                    )
                    .await
                {
                    error!("Connection error: {}", err);
                }
            });
        }
    }

    async fn handle_request(
        req: Request<hyper::body::Incoming>,
        connection_manager: Arc<ConnectionManager>,
        manifest_manager: Arc<ManifestManager>,
        base_url: Arc<String>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        let response = match req.method() {
            &Method::POST => match req.uri().path() {
                "/mcp" => {
                    Self::handle_mcp_post(req, connection_manager, manifest_manager, base_url).await
                }
                "/status" => Self::handle_status(connection_manager).await,
                _ => Ok(Self::not_found_response()),
            },
            &Method::GET => match req.uri().path() {
                "/status" => Self::handle_status(connection_manager).await,
                "/health" => Ok(Self::health_response()),
                _ => Ok(Self::not_found_response()),
            },
            &Method::OPTIONS => Ok(Self::cors_response()),
            _ => Ok(Self::not_found_response()),
        };

        response
    }

    async fn handle_mcp_post(
        req: Request<hyper::body::Incoming>,
        connection_manager: Arc<ConnectionManager>,
        manifest_manager: Arc<ManifestManager>,
        base_url: Arc<String>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        let headers = req.headers().clone();
        let body_bytes = req.collect().await?.to_bytes();
        let body_str = String::from_utf8_lossy(&body_bytes);

        debug!("Received MCP request: {}", body_str);

        let request: McpRequest = match serde_json::from_str(&body_str) {
            Ok(req) => req,
            Err(e) => {
                error!("Failed to parse MCP request: {}", e);
                let detailed_error = format!(
                    "JSON parse error: {}. Check your JSON syntax - you may have missing quotes, extra commas, or malformed structure.", 
                    e
                );
                return Ok(Self::error_response(-32700, &detailed_error));
            }
        };

        let response = match request.method.as_str() {
            "initialize" => Self::handle_initialize(&request).await,
            "notifications/initialized" => {
                // Handle initialized notification - keep connection open for SSE
                info!("Received initialized notification from client");
                info!("Request headers: {:?}", headers);

                // Return SSE stream that stays open
                return Ok(Self::sse_stream_response());
            }
            "tools/list" => {
                Self::handle_tools_list(&request, &connection_manager, &manifest_manager).await
            }
            "tools/call" => {
                Self::handle_tools_call(&request, &connection_manager, &manifest_manager, &base_url)
                    .await
            }
            _ => McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(McpError {
                    code: -32601,
                    message: "Method not found".to_string(),
                    data: None,
                }),
            },
        };

        let response_json = serde_json::to_string(&response).unwrap();
        debug!("Sending MCP response: {}", response_json);

        Ok(Self::json_response(response_json))
    }

    async fn handle_status(
        connection_manager: Arc<ConnectionManager>,
    ) -> Result<Response<BoxBody<hyper::body::Bytes, hyper::Error>>, hyper::Error> {
        let state = connection_manager.get_state();

        let status = serde_json::json!({
            "state": format!("{:?}", state),
            "message": state.error_message(),
            "device_id": state.device_id(),
            "ready": state.is_ready()
        });

        Ok(Self::json_response(serde_json::to_string(&status).unwrap()))
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

    async fn handle_tools_list(
        _request: &McpRequest,
        connection_manager: &Arc<ConnectionManager>,
        manifest_manager: &Arc<ManifestManager>,
    ) -> McpResponse {
        let state = connection_manager.get_state();

        match state.device_id() {
            Some(device_id) => match manifest_manager.get_manifest(device_id) {
                Ok(manifest) => {
                    let mut tools = manifest_manager.create_tools_list(&manifest);
                    tools.push(Self::python_runner_tool());

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
                Err(e) => McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: _request.id.clone(),
                    result: None,
                    error: Some(McpError {
                        code: -32603,
                        message: format!("Failed to load manifest: {}", e),
                        data: None,
                    }),
                },
            },
            None => {
                // Return empty tools list with status info
                let result = serde_json::json!({
                    "tools": [],
                    "_status": {
                        "robot_state": format!("{:?}", state),
                        "message": state.error_message()
                    }
                });

                McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: _request.id.clone(),
                    result: Some(result),
                    error: None,
                }
            }
        }
    }

    async fn handle_tools_call(
        request: &McpRequest,
        connection_manager: &Arc<ConnectionManager>,
        manifest_manager: &Arc<ManifestManager>,
        base_url: &Arc<String>,
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

        // Check robot state first
        let state = connection_manager.get_state();
        if !state.is_ready() {
            return McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: None,
                error: Some(McpError {
                    code: -32603,
                    message: format!("Robot not ready: {}", state.error_message()),
                    data: Some(serde_json::json!({
                        "robot_state": format!("{:?}", state),
                        "suggestion": "Check robot connection and try again"
                    })),
                }),
            };
        }

        let device_id = state.device_id().unwrap(); // Safe because state.is_ready()

        // Get manifest and find function
        let manifest = match manifest_manager.get_manifest(device_id) {
            Ok(m) => m,
            Err(e) => {
                return McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id.clone(),
                    result: None,
                    error: Some(McpError {
                        code: -32603,
                        message: format!("Failed to load manifest: {}", e),
                        data: None,
                    }),
                };
            }
        };

        if tool_name == "runPythonScript" {
            return Self::handle_run_python_script(request, arguments, &manifest, base_url).await;
        }

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

        // Validate arguments
        if let Err(e) = manifest_manager.validate_function_arguments(func, arguments) {
            return McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: None,
                error: Some(McpError {
                    code: -32602,
                    message: format!("Invalid arguments: {}", e),
                    data: None,
                }),
            };
        }

        // Execute the function
        match connection_manager.execute_function(func, arguments) {
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
            Err(e) => McpResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id.clone(),
                result: None,
                error: Some(McpError {
                    code: -32603,
                    message: format!("Execution error: {}", e),
                    data: Some(serde_json::json!({
                        "robot_state": format!("{:?}", connection_manager.get_state()),
                        "suggestion": "Check robot connection and try again"
                    })),
                }),
            },
        }
    }

    async fn handle_run_python_script(
        request: &McpRequest,
        arguments: &Value,
        manifest: &Manifest,
        base_url: &Arc<String>,
    ) -> McpResponse {
        let script_value = match arguments.get("script") {
            Some(value) => value,
            None => {
                return McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id.clone(),
                    result: None,
                    error: Some(McpError {
                        code: -32602,
                        message: "Missing required parameter 'script' for runPythonScript"
                            .to_string(),
                        data: None,
                    }),
                };
            }
        };

        let script = match script_value.as_str() {
            Some(value) => value,
            None => {
                return McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id.clone(),
                    result: None,
                    error: Some(McpError {
                        code: -32602,
                        message: "Parameter 'script' must be a string".to_string(),
                        data: None,
                    }),
                };
            }
        };

        let timeout_secs = match arguments.get("timeout") {
            Some(value) => {
                if let Some(num) = value.as_u64() {
                    if num == 0 {
                        return McpResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id.clone(),
                            result: None,
                            error: Some(McpError {
                                code: -32602,
                                message: "Parameter 'timeout' must be greater than 0 seconds"
                                    .to_string(),
                                data: None,
                            }),
                        };
                    }
                    if num > 300 {
                        return McpResponse {
                            jsonrpc: "2.0".to_string(),
                            id: request.id.clone(),
                            result: None,
                            error: Some(McpError {
                                code: -32602,
                                message: "Parameter 'timeout' cannot exceed 300 seconds"
                                    .to_string(),
                                data: None,
                            }),
                        };
                    }
                    num
                } else {
                    return McpResponse {
                        jsonrpc: "2.0".to_string(),
                        id: request.id.clone(),
                        result: None,
                        error: Some(McpError {
                            code: -32602,
                            message: "Parameter 'timeout' must be an integer number of seconds"
                                .to_string(),
                            data: None,
                        }),
                    };
                }
            }
            None => 60,
        };

        let mut tool_names: Vec<String> =
            manifest.functions.iter().map(|f| f.name.clone()).collect();
        if !tool_names.iter().any(|name| name == "runPythonScript") {
            tool_names.push("runPythonScript".to_string());
        }

        match python_runner::run_python_script(script, timeout_secs, &tool_names, base_url.as_str())
            .await
        {
            Ok(output) => {
                let result = serde_json::json!({
                    "content": [
                        {
                            "type": "text",
                            "text": output
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
            Err(err) => {
                error!("runPythonScript failed: {}", err);
                McpResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id.clone(),
                    result: None,
                    error: Some(McpError {
                        code: -32603,
                        message: format!("Failed to execute Python script: {}", err),
                        data: None,
                    }),
                }
            }
        }
    }

    fn python_runner_tool() -> Tool {
        Tool {
            name: "runPythonScript".to_string(),
            description: "Execute a Python 3 script with access to the robot tools namespace. Use this when you need loops, conditionals, or batching before invoking MCP tools. Inside the script call other tools as `tools.FUNCNAME(arg=value, ...)`. The combined console output is returned as text.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "script": {
                        "type": "string",
                        "description": "Python 3 source code to execute. Use the provided `tools` namespace to call MCP functions."
                    },
                    "timeout": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 300,
                        "default": 60,
                        "description": "Optional timeout in seconds (default 60, maximum 300)."
                    }
                },
                "required": ["script"]
            }),
        }
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
            .body(BoxBody::new(
                Full::new("Not Found".into()).map_err(|e| match e {}),
            ))
            .unwrap()
    }

    fn health_response() -> Response<BoxBody<hyper::body::Bytes, hyper::Error>> {
        let health = serde_json::json!({
            "status": "ok",
            "service": "arduino-mcp-adapter",
            "version": "0.1.0"
        });
        Self::json_response(serde_json::to_string(&health).unwrap())
    }

    fn error_response(
        code: i32,
        message: &str,
    ) -> Response<BoxBody<hyper::body::Bytes, hyper::Error>> {
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
        Self::json_response(body)
    }

    fn empty_response() -> Response<BoxBody<hyper::body::Bytes, hyper::Error>> {
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .header("Access-Control-Allow-Origin", "*")
            .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            .header("Access-Control-Allow-Headers", "Content-Type")
            .body(BoxBody::new(Full::new("{}".into()).map_err(|e| match e {})))
            .unwrap()
    }

    fn sse_stream_response() -> Response<BoxBody<hyper::body::Bytes, hyper::Error>> {
        use tokio_stream::wrappers::ReceiverStream;

        // Create a channel and spawn a task to keep the sender alive indefinitely
        let (tx, rx) = tokio::sync::mpsc::channel::<
            Result<hyper::body::Frame<hyper::body::Bytes>, hyper::Error>,
        >(1);

        // Spawn a task that holds the sender forever, keeping the stream alive
        tokio::spawn(async move {
            let _tx = tx; // Keep sender alive
                          // Sleep forever - this keeps the connection open
            std::future::pending::<()>().await;
        });

        let stream = ReceiverStream::new(rx);

        Response::builder()
            .status(StatusCode::ACCEPTED)
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .header("Access-Control-Allow-Origin", "*")
            .body(BoxBody::new(http_body_util::StreamBody::new(stream)))
            .unwrap()
    }
}
