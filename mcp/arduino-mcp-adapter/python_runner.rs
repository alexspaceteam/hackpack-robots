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
    timeout_secs: u64,
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

    let timeout_duration = Duration::from_secs(timeout_secs);
    let output = match time::timeout(timeout_duration, child.wait_with_output()).await {
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
    let mut prelude = format!(
        r#"import json
import urllib.request
import urllib.error

MCP_ENDPOINT = {endpoint:?}


class _ToolsNamespace:
    def __init__(self, endpoint):
        self._endpoint = endpoint
        self._call_id = 0

    def _call(self, name, **kwargs):
        self._call_id += 1
        payload = {{
            "jsonrpc": "2.0",
            "id": f"python-runner-{{self._call_id}}",
            "method": "tools/call",
            "params": {{
                "name": name,
                "arguments": kwargs,
            }},
        }}

        data = json.dumps(payload).encode("utf-8")
        request = urllib.request.Request(
            self._endpoint,
            data=data,
            headers={{"Content-Type": "application/json"}},
            method="POST",
        )

        try:
            with urllib.request.urlopen(request, timeout=60) as response:
                response_data = response.read().decode("utf-8")
        except urllib.error.HTTPError as exc:
            body = exc.read().decode("utf-8", errors="ignore")
            raise RuntimeError(f"MCP HTTP error calling {{name}}: {{exc.code}} {{body}}") from exc
        except urllib.error.URLError as exc:
            raise RuntimeError(f"Failed to reach MCP endpoint for {{name}}: {{exc}}") from exc

        message = json.loads(response_data)
        if message.get("error"):
            err = message["error"]
            raise RuntimeError(
                f"MCP error calling {{name}}: {{err.get('message')}} (code {{err.get('code')}})"
            )

        result = message.get("result") or {{}}
        content = result.get("content") if isinstance(result, dict) else None
        if isinstance(content, list):
            texts = [item.get("text", "") for item in content if item.get("type") == "text"]
            if len(texts) == 1:
                return texts[0]
            if texts:
                return "\n".join(texts)
        return result


tools = _ToolsNamespace(MCP_ENDPOINT)


def _wrap_tool(name):
    def _inner(**kwargs):
        return tools._call(name, **kwargs)
    _inner.__name__ = name
    return _inner

"#,
        endpoint = endpoint
    );

    for (index, name) in tool_names.iter().enumerate() {
        let helper_name = format!("_tool_fn_{}", index);
        prelude.push_str(&format!(
            "{helper} = _wrap_tool({name_literal})\nsetattr(tools, {name_literal}, {helper})\n\n",
            helper = helper_name,
            name_literal = serde_json::to_string(name).unwrap()
        ));
    }

    prelude
}
