use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::{info, warn, debug};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Manifest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub functions: Vec<Function>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Function {
    pub tag: u8,
    pub name: String,
    pub desc: String,
    #[serde(rename = "return")]
    pub return_type: Option<String>,
    pub params: Vec<Parameter>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

pub struct ManifestManager {
    manifest_dir: PathBuf,
    loaded_manifests: Arc<Mutex<HashMap<String, Manifest>>>,
}

impl ManifestManager {
    pub fn new(manifest_dir: PathBuf) -> Self {
        Self {
            manifest_dir,
            loaded_manifests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get_manifest(&self, device_id: &str) -> Result<Manifest> {
        // Check if already loaded
        {
            let manifests = self.loaded_manifests.lock().unwrap();
            if let Some(manifest) = manifests.get(device_id) {
                debug!("Using cached manifest for device: {}", device_id);
                return Ok(manifest.clone());
            }
        }

        // Load from disk
        let manifest_path = self.manifest_dir.join(format!("{}.json", device_id));
        info!("Loading manifest from: {}", manifest_path.display());
        
        if !manifest_path.exists() {
            return Err(anyhow!(
                "Manifest not found for device '{}'. Expected file: {}. Make sure the manifest file exists and the device ID is correct.",
                device_id,
                manifest_path.display()
            ));
        }

        let manifest = self.load_manifest_from_file(&manifest_path)?;
        
        // Cache the loaded manifest
        {
            let mut manifests = self.loaded_manifests.lock().unwrap();
            manifests.insert(device_id.to_string(), manifest.clone());
        }
        
        info!("Loaded manifest for {}: {} (version: {})", device_id, manifest.name, manifest.version);
        info!("Available functions: {}", manifest.functions.len());
        
        // Log function details
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
        
        Ok(manifest)
    }

    pub fn reload_manifest(&self, device_id: &str) -> Result<()> {
        info!("Reloading manifest for device: {}", device_id);
        
        // Remove from cache
        {
            let mut manifests = self.loaded_manifests.lock().unwrap();
            manifests.remove(device_id);
        }
        
        // Load fresh copy
        self.get_manifest(device_id)?;
        
        Ok(())
    }

    pub fn list_available_manifests(&self) -> Result<Vec<String>> {
        let mut device_ids = Vec::new();
        
        if !self.manifest_dir.exists() {
            warn!("Manifest directory does not exist: {}", self.manifest_dir.display());
            return Ok(device_ids);
        }

        let entries = std::fs::read_dir(&self.manifest_dir)?;
        
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            
            if path.is_file() && path.extension().map_or(false, |ext| ext == "json") {
                if let Some(stem) = path.file_stem() {
                    if let Some(device_id) = stem.to_str() {
                        device_ids.push(device_id.to_string());
                    }
                }
            }
        }
        
        device_ids.sort();
        info!("Available manifest files: {:?}", device_ids);
        
        Ok(device_ids)
    }

    pub fn validate_function_arguments(&self, func: &Function, arguments: &Value) -> Result<()> {
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

    pub fn create_tools_list(&self, manifest: &Manifest) -> Vec<Tool> {
        manifest.functions.iter().map(|func| {
            Tool {
                name: func.name.clone(),
                description: func.desc.clone(),
                input_schema: self.create_input_schema(func),
            }
        }).collect()
    }

    fn create_input_schema(&self, func: &Function) -> Value {
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

    fn load_manifest_from_file(&self, path: &PathBuf) -> Result<Manifest> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read manifest file {}: {}", path.display(), e))?;
        
        let manifest: Manifest = serde_json::from_str(&content)
            .map_err(|e| anyhow!("Failed to parse manifest file {}: {}", path.display(), e))?;
        
        Ok(manifest)
    }
}

pub fn type_to_json_type(rust_type: &str) -> &'static str {
    match rust_type {
        "i16" | "i32" | "i64" => "integer",
        "f32" | "f64" => "number",
        "CStr" => "string",
        "bool" => "boolean",
        _ => "string", // Default fallback
    }
}