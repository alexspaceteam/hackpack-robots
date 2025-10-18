use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tempfile::Builder;
use tokio::process::Command;
use tokio::time;

/// Execute the provided Python script with a prelude that exposes MCP tools.
pub async fn run_python_script(
    script: &str,
    timeout: Duration,
    tool_names: &[String],
    endpoint: &str,
) -> Result<String> {
    if script.trim().is_empty() {
        return Err(anyhow!("Python script must not be empty"));
    }

    let mut full_script = build_prelude(tool_names, endpoint);
    full_script.push_str("\n# --- User script starts here ---\n");
    full_script.push_str(script);
    if !script.ends_with('\n') {
        full_script.push('\n');
    }

    let mut temp_file = Builder::new()
        .prefix("arduino-mcp-script-")
        .suffix(".py")
        .tempfile()
        .context("Failed to create temporary Python file")?;
    temp_file
        .write_all(full_script.as_bytes())
        .context("Failed to write temporary Python script")?;

    let temp_path = temp_file.into_temp_path();
    let script_path: PathBuf = temp_path.to_path_buf();

    let mut command = Command::new("python3");
    command.arg(&script_path);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);

    let child = command
        .spawn()
        .context("Failed to spawn python3 process. Ensure python3 is installed and on PATH.")?;

    let timeout_secs = timeout.as_secs();
    let output = match time::timeout(timeout, child.wait_with_output()).await {
        Ok(result) => result.context("Failed to collect python3 output")?,
        Err(_) => {
            return Err(anyhow!(
                "Python script timed out after {} seconds",
                timeout_secs
            ));
        }
    };

    // Drop the temp path to ensure the file is removed after execution
    drop(temp_path);

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        let status_str = match output.status.code() {
            Some(code) => format!("exit code {}", code),
            None => "terminated by signal".to_string(),
        };

        return Err(anyhow!(
            "Python script failed with {}.\nSTDOUT:\n{}\nSTDERR:\n{}",
            status_str,
            stdout,
            stderr
        ));
    }

    Ok(format_console_output(stdout, stderr))
}

fn format_console_output(stdout: String, stderr: String) -> String {
    let stdout_trimmed = stdout.trim_end_matches('\n');
    let stderr_trimmed = stderr.trim_end_matches('\n');

    let has_stdout = !stdout_trimmed.is_empty();
    let has_stderr = !stderr_trimmed.is_empty();

    match (has_stdout, has_stderr) {
        (false, false) => "Python script completed without console output.".to_string(),
        (true, false) => stdout_trimmed.to_string(),
        (false, true) => format!("[stderr]\n{}", stderr_trimmed),
        (true, true) => format!("{}\n[stderr]\n{}", stdout_trimmed, stderr_trimmed),
    }
}

fn build_prelude(tool_names: &[String], endpoint: &str) -> String {
    const TEMPLATE: &str = include_str!("resources/python_prelude.py.tmpl");

    let endpoint_literal = serde_json::to_string(endpoint).unwrap();
    let trampolines = tool_names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let name_literal = serde_json::to_string(name).unwrap();
            format!(
                "_tool_fn_{idx} = _wrap_tool({name_literal})\nsetattr(tools, {name_literal}, _tool_fn_{idx})",
                idx = index,
                name_literal = name_literal
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    TEMPLATE
        .replace("__MCP_ENDPOINT__", &endpoint_literal)
        .replace("__TOOL_TRAMPOLINES__", &trampolines)
}
