use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use uuid::Uuid;
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};

// Discord Application ID - replace with your actual ID from Discord Developer Portal
const DISCORD_APP_ID: &str = "1448073023348539495";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserScript {
    id: String,
    name: String,
    code: String,
    enabled: bool,
    #[serde(default)]
    order: i32,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    last_updated: Option<u64>,
    #[serde(default)]
    last_fetch_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScriptDependency {
    url: String,
    code: String,
    last_updated: u64,
}

struct AppState {
    scripts: Mutex<Vec<UserScript>>,
    dependencies: Mutex<HashMap<String, ScriptDependency>>,
    data_dir: PathBuf,
    discord_client: Mutex<Option<DiscordIpcClient>>,
}

impl AppState {
    fn new() -> Self {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("GeoGuessrDesktop");

        fs::create_dir_all(&data_dir).ok();

        let scripts_file = data_dir.join("scripts.json");
        let scripts = if scripts_file.exists() {
            let content = fs::read_to_string(&scripts_file).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        };

        let dependencies_file = data_dir.join("dependencies.json");
        let dependencies = if dependencies_file.exists() {
            let content = fs::read_to_string(&dependencies_file).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashMap::new()
        };

        AppState {
            scripts: Mutex::new(scripts),
            dependencies: Mutex::new(dependencies),
            data_dir,
            discord_client: Mutex::new(None),
        }
    }

    fn save_scripts(&self, scripts: &[UserScript]) -> Result<(), String> {
        let scripts_file = self.data_dir.join("scripts.json");
        let content = serde_json::to_string_pretty(scripts)
            .map_err(|e| format!("Failed to serialize scripts: {}", e))?;
        fs::write(&scripts_file, content)
            .map_err(|e| format!("Failed to write scripts file: {}", e))?;
        Ok(())
    }

    fn save_dependencies(&self, dependencies: &HashMap<String, ScriptDependency>) -> Result<(), String> {
        let dependencies_file = self.data_dir.join("dependencies.json");
        let content = serde_json::to_string_pretty(dependencies)
            .map_err(|e| format!("Failed to serialize dependencies: {}", e))?;
        fs::write(&dependencies_file, content)
            .map_err(|e| format!("Failed to write dependencies file: {}", e))?;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct ScriptMetadata {
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    author: Option<String>,
    requires: Vec<String>,
}

fn parse_metadata(code: &str) -> ScriptMetadata {
    use regex::Regex;

    let mut metadata = ScriptMetadata::default();

    // Extract metadata block between // ==UserScript== and // ==/UserScript==
    let metadata_regex = Regex::new(r"(?s)//\s*==UserScript==(.*?)//\s*==/UserScript==").unwrap();

    if let Some(captures) = metadata_regex.captures(code) {
        if let Some(metadata_block) = captures.get(1) {
            let block = metadata_block.as_str();

            // Parse @name
            if let Some(caps) = Regex::new(r"@name\s+(.+)").unwrap().captures(block) {
                metadata.name = caps.get(1).map(|m| m.as_str().trim().to_string());
            }

            // Parse @version
            if let Some(caps) = Regex::new(r"@version\s+(.+)").unwrap().captures(block) {
                metadata.version = caps.get(1).map(|m| m.as_str().trim().to_string());
            }

            // Parse @description
            if let Some(caps) = Regex::new(r"@description\s+(.+)").unwrap().captures(block) {
                metadata.description = caps.get(1).map(|m| m.as_str().trim().to_string());
            }

            // Parse @author
            if let Some(caps) = Regex::new(r"@author\s+(.+)").unwrap().captures(block) {
                metadata.author = caps.get(1).map(|m| m.as_str().trim().to_string());
            }

            // Parse @require (can appear multiple times)
            let require_regex = Regex::new(r"@require\s+(https?://\S+)").unwrap();
            for caps in require_regex.captures_iter(block) {
                if let Some(url) = caps.get(1) {
                    metadata.requires.push(url.as_str().trim().to_string());
                }
            }
        }
    }

    metadata
}

fn fetch_script_from_url(url: &str) -> Result<String, String> {
    use reqwest::blocking::Client;
    use std::time::Duration;

    // Validate URL starts with https
    if !url.starts_with("https://") {
        return Err("Only HTTPS URLs are supported for security reasons".to_string());
    }

    // Create HTTP client with timeout
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Fetch the script
    let response = client
        .get(url)
        .header("User-Agent", "GeoGuessrDesktop/1.0")
        .send()
        .map_err(|e| {
            if e.is_timeout() {
                "Request timed out after 30 seconds".to_string()
            } else if e.is_connect() {
                format!("Failed to connect to {}", url)
            } else {
                format!("Network error: {}", e)
            }
        })?;

    // Check status code
    if !response.status().is_success() {
        return Err(format!("HTTP {}: {}", response.status().as_u16(), response.status().canonical_reason().unwrap_or("Unknown error")));
    }

    // Check content type
    if let Some(content_type) = response.headers().get("content-type") {
        let content_type_str = content_type.to_str().unwrap_or("");
        if !content_type_str.contains("javascript") && !content_type_str.contains("text/plain") {
            return Err(format!("Expected JavaScript, got content-type: {}", content_type_str));
        }
    }

    // Get response body
    let body = response.text().map_err(|e| format!("Failed to read response: {}", e))?;

    // Check size (10MB limit)
    if body.len() > 10 * 1024 * 1024 {
        return Err("Script too large (>10MB)".to_string());
    }

    Ok(body)
}

fn fetch_script_with_dependencies(
    url: &str,
    dependency_cache: &mut HashMap<String, ScriptDependency>
) -> Result<UserScript, String> {
    use chrono::Utc;

    // Fetch main script
    let code = fetch_script_from_url(url)?;

    // Parse metadata
    let metadata = parse_metadata(&code);

    // Fetch dependencies
    for dep_url in &metadata.requires {
        // Check if already in cache
        if !dependency_cache.contains_key(dep_url) {
            // Fetch dependency
            match fetch_script_from_url(dep_url) {
                Ok(dep_code) => {
                    let dependency = ScriptDependency {
                        url: dep_url.clone(),
                        code: dep_code,
                        last_updated: Utc::now().timestamp() as u64,
                    };
                    dependency_cache.insert(dep_url.clone(), dependency);
                }
                Err(e) => {
                    return Err(format!("Failed to fetch dependency {}: {}", dep_url, e));
                }
            }
        }
    }

    // Create UserScript
    let script = UserScript {
        id: Uuid::new_v4().to_string(),
        name: metadata.name.unwrap_or_else(|| "Unnamed Script".to_string()),
        code,
        enabled: true,
        order: 0, // Will be set by add_script_from_url
        url: Some(url.to_string()),
        version: metadata.version,
        description: metadata.description,
        author: metadata.author,
        requires: metadata.requires,
        last_updated: Some(Utc::now().timestamp() as u64),
        last_fetch_error: None,
    };

    Ok(script)
}

#[tauri::command]
fn get_scripts(state: tauri::State<AppState>) -> Result<Vec<UserScript>, String> {
    let scripts = state.scripts.lock().unwrap();
    Ok(scripts.clone())
}

#[tauri::command]
fn add_script_from_url(url: String, state: tauri::State<AppState>) -> Result<UserScript, String> {
    let mut scripts = state.scripts.lock().unwrap();
    let mut dependencies = state.dependencies.lock().unwrap();

    // Check for duplicate URLs
    if scripts.iter().any(|s| s.url.as_ref() == Some(&url)) {
        return Err("A script from this URL already exists".to_string());
    }

    // Fetch script with dependencies
    let mut new_script = fetch_script_with_dependencies(&url, &mut dependencies)?;

    // Assign order (highest + 1)
    let max_order = scripts.iter().map(|s| s.order).max().unwrap_or(-1);
    new_script.order = max_order + 1;

    // Save
    scripts.push(new_script.clone());
    let scripts_clone = scripts.clone();
    let dependencies_clone = dependencies.clone();
    drop(scripts); // Release lock before saving
    drop(dependencies);
    state.save_scripts(&scripts_clone)?;
    state.save_dependencies(&dependencies_clone)?;

    Ok(new_script)
}

#[tauri::command]
fn reorder_script(id: String, new_order: i32, state: tauri::State<AppState>) -> Result<(), String> {
    let mut scripts = state.scripts.lock().unwrap();

    if let Some(script) = scripts.iter_mut().find(|s| s.id == id) {
        script.order = new_order;
        state.save_scripts(&scripts)?;
        Ok(())
    } else {
        Err("Script not found".to_string())
    }
}

#[tauri::command]
fn toggle_script(id: String, enabled: bool, state: tauri::State<AppState>) -> Result<(), String> {
    let mut scripts = state.scripts.lock().unwrap();

    if let Some(script) = scripts.iter_mut().find(|s| s.id == id) {
        script.enabled = enabled;
        state.save_scripts(&scripts)?;
        Ok(())
    } else {
        Err("Script not found".to_string())
    }
}

#[tauri::command]
fn delete_script(id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let mut scripts = state.scripts.lock().unwrap();
    scripts.retain(|s| s.id != id);
    state.save_scripts(&scripts)?;
    Ok(())
}

#[tauri::command]
fn refresh_script(id: String, state: tauri::State<AppState>) -> Result<UserScript, String> {
    use chrono::Utc;

    let mut scripts = state.scripts.lock().unwrap();
    let mut dependencies = state.dependencies.lock().unwrap();

    // Find script
    let script_index = scripts.iter().position(|s| s.id == id)
        .ok_or_else(|| "Script not found".to_string())?;

    let script = &scripts[script_index];

    // Check if script has URL
    let url = script.url.as_ref()
        .ok_or_else(|| "Cannot refresh manually added script".to_string())?;

    // Preserve user settings
    let preserved_enabled = script.enabled;
    let preserved_order = script.order;
    let preserved_id = script.id.clone();

    // Fetch fresh copy
    let mut updated_script = fetch_script_with_dependencies(url, &mut dependencies)?;

    // Restore user settings
    updated_script.id = preserved_id;
    updated_script.enabled = preserved_enabled;
    updated_script.order = preserved_order;
    updated_script.last_updated = Some(Utc::now().timestamp() as u64);
    updated_script.last_fetch_error = None;

    // Update in list
    scripts[script_index] = updated_script.clone();

    let scripts_clone = scripts.clone();
    let dependencies_clone = dependencies.clone();
    drop(scripts);
    drop(dependencies);
    state.save_scripts(&scripts_clone)?;
    state.save_dependencies(&dependencies_clone)?;

    Ok(updated_script)
}

#[tauri::command]
fn auto_update_scripts(state: tauri::State<AppState>) -> Result<usize, String> {
    use chrono::Utc;

    let mut scripts = state.scripts.lock().unwrap();
    let mut dependencies = state.dependencies.lock().unwrap();

    let now = Utc::now().timestamp() as u64;
    let one_day = 24 * 60 * 60;
    let one_hour = 60 * 60;
    let mut updated_count = 0;

    for script in scripts.iter_mut() {
        // Only update scripts with URLs
        if let Some(url) = &script.url {
            // Skip if updated recently (< 24 hours)
            if let Some(last_updated) = script.last_updated {
                if now - last_updated < one_day {
                    continue;
                }
            }

            // Skip if recent error (< 1 hour)
            if script.last_fetch_error.is_some() {
                if let Some(last_updated) = script.last_updated {
                    if now - last_updated < one_hour {
                        continue;
                    }
                }
            }

            // Try to fetch update
            match fetch_script_with_dependencies(url, &mut dependencies) {
                Ok(updated) => {
                    // Preserve user settings
                    script.code = updated.code;
                    script.name = updated.name;
                    script.version = updated.version;
                    script.description = updated.description;
                    script.author = updated.author;
                    script.requires = updated.requires;
                    script.last_updated = Some(now);
                    script.last_fetch_error = None;
                    updated_count += 1;
                }
                Err(e) => {
                    script.last_fetch_error = Some(e);
                    script.last_updated = Some(now);
                }
            }
        }
    }

    let scripts_clone = scripts.clone();
    let dependencies_clone = dependencies.clone();
    drop(scripts);
    drop(dependencies);
    state.save_scripts(&scripts_clone)?;
    state.save_dependencies(&dependencies_clone)?;

    Ok(updated_count)
}

#[tauri::command]
fn get_data_dir(state: tauri::State<AppState>) -> Result<String, String> {
    Ok(state.data_dir.to_string_lossy().to_string())
}

// Discord Rich Presence commands
#[tauri::command]
async fn discord_connect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Run blocking Discord IPC connection in a separate thread
    let client_result = tokio::task::spawn_blocking(move || {
        let mut client = DiscordIpcClient::new(DISCORD_APP_ID);
        client.connect()
            .map_err(|e| format!("Failed to connect to Discord: {}", e))?;
        Ok::<_, String>(client)
    }).await.map_err(|e| format!("Task error: {}", e))?;

    match client_result {
        Ok(client) => {
            *state.discord_client.lock().unwrap() = Some(client);
            println!("[Discord] Connected to Discord RPC");
            Ok(())
        }
        Err(e) => {
            println!("[Discord] Connection failed: {}", e);
            Err(e)
        }
    }
}

#[tauri::command]
fn discord_update_presence(
    details: Option<String>,
    presence_state: Option<String>,
    start_timestamp: Option<i64>,
    state: tauri::State<'_, AppState>
) -> Result<(), String> {
    let mut guard = state.discord_client.lock().unwrap();
    if let Some(client) = guard.as_mut() {
        let mut act = activity::Activity::new();

        if let Some(d) = &details {
            act = act.details(d);
        }
        if let Some(s) = &presence_state {
            act = act.state(s);
        }
        if let Some(ts) = start_timestamp {
            act = act.timestamps(activity::Timestamps::new().start(ts));
        }

        // Add assets (you can configure these in Discord Developer Portal)
        act = act.assets(
            activity::Assets::new()
                .large_image("geoguessr_logo")
                .large_text("GeoGuessr Desktop")
        );

        client.set_activity(act)
            .map_err(|e| format!("Failed to set activity: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
fn discord_clear_presence(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.discord_client.lock().unwrap();
    if let Some(client) = guard.as_mut() {
        client.clear_activity()
            .map_err(|e| format!("Failed to clear activity: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
fn discord_disconnect(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.discord_client.lock().unwrap();
    if let Some(mut client) = guard.take() {
        client.close()
            .map_err(|e| format!("Failed to disconnect from Discord: {}", e))?;
        println!("[Discord] Disconnected from Discord RPC");
    }
    Ok(())
}

#[tauri::command]
async fn open_geoguessr(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    if let Some(_window) = app.get_webview_window("geoguessr") {
        return Ok(());
    }

    // Get all enabled scripts and combine them
    let init_script = get_initialization_script(&state);

    let _window = WebviewWindowBuilder::new(&app, "geoguessr", WebviewUrl::External("https://www.geoguessr.com/".parse().unwrap()))
        .title("GeoGuessr Desktop")
        .inner_size(1400.0, 900.0)
        .resizable(true)
        .decorations(false) // Custom titlebar
        .initialization_script(&init_script)
        .on_navigation(move |url| {
            // Allow navigation to geoguessr.com domains
            url.host_str() == Some("www.geoguessr.com") ||
            url.host_str() == Some("geoguessr.com")
        })
        .build()
        .map_err(|e| format!("Failed to create window: {}", e))?;

    Ok(())
}

fn get_initialization_script(state: &AppState) -> String {
    use std::collections::HashSet;

    let scripts = state.scripts.lock().unwrap();
    let dependencies = state.dependencies.lock().unwrap();
    let mut enabled_scripts: Vec<_> = scripts.iter().filter(|s| s.enabled).collect();

    // Sort scripts by order (lower numbers load first)
    enabled_scripts.sort_by_key(|s| s.order);

    // Build JSON list of all scripts for settings panel
    let all_scripts_json = serde_json::to_string(&*scripts).unwrap_or_else(|_| "[]".to_string());

    let mut combined = String::new();

    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    // Inject scripts into page's main world via script tags
    // This is critical - initialization_script runs in isolated context,
    // but we need to run in the page's main world to intercept fetch
    combined.push_str("(function() {\n");
    combined.push_str("  // Only run in main frame, not iframes\n");
    combined.push_str("  if (window !== window.top) return;\n");
    combined.push_str("  \n");
    combined.push_str("  // Prevent multiple executions\n");
    combined.push_str("  if (window.__geoguessrDesktopInjected) return;\n");
    combined.push_str("  window.__geoguessrDesktopInjected = true;\n");
    combined.push_str("  \n");
    combined.push_str("  console.log('[GeoGuessr Desktop] Initializing userscripts...');\n\n");

    // Base64 decode helper - this runs in isolated context
    combined.push_str("  function decodeBase64(str) {\n");
    combined.push_str("    return decodeURIComponent(atob(str).split('').map(function(c) {\n");
    combined.push_str("      return '%' + ('00' + c.charCodeAt(0).toString(16)).slice(-2);\n");
    combined.push_str("    }).join(''));\n");
    combined.push_str("  }\n\n");

    // Inject and run script in page's main world via script tag
    // This MUST use script tags to escape the isolated context
    combined.push_str("  function injectIntoPage(code, name) {\n");
    combined.push_str("    console.log('[GeoGuessr Desktop] Injecting into page:', name);\n");
    combined.push_str("    var script = document.createElement('script');\n");
    combined.push_str("    script.textContent = code;\n");
    combined.push_str("    script.setAttribute('data-geoguessr-desktop', name || 'userscript');\n");
    combined.push_str("    // Inject into documentElement (exists before head/body)\n");
    combined.push_str("    document.documentElement.appendChild(script);\n");
    combined.push_str("    // Clean up immediately after execution\n");
    combined.push_str("    script.remove();\n");
    combined.push_str("  }\n\n");

    // Wait for documentElement to exist, then inject all scripts
    combined.push_str("  function waitForDocumentElement(callback) {\n");
    combined.push_str("    if (document.documentElement) {\n");
    combined.push_str("      callback();\n");
    combined.push_str("    } else {\n");
    combined.push_str("      // Poll rapidly until documentElement exists\n");
    combined.push_str("      var interval = setInterval(function() {\n");
    combined.push_str("        if (document.documentElement) {\n");
    combined.push_str("          clearInterval(interval);\n");
    combined.push_str("          callback();\n");
    combined.push_str("        }\n");
    combined.push_str("      }, 1);\n");
    combined.push_str("    }\n");
    combined.push_str("  }\n\n");

    combined.push_str("  waitForDocumentElement(function() {\n");
    combined.push_str("    console.log('[GeoGuessr Desktop] Document element ready, injecting scripts...');\n\n");

    // Tampermonkey API - encode as base64
    // Put everything on window so it's accessible across all script injections
    let tampermonkey_api = r#"
window.unsafeWindow = window;
window.GM_info = {
  script: { name: 'GeoGuessr Desktop', version: '1.0' },
  scriptHandler: 'GeoGuessr Desktop',
  version: '1.0'
};
window.GM_getValue = function(key, defaultValue) {
  try {
    var value = localStorage.getItem('gm_' + key);
    return value !== null ? JSON.parse(value) : defaultValue;
  } catch(e) {
    console.warn('[GM_getValue] Error:', e);
    return defaultValue;
  }
};
window.GM_setValue = function(key, value) {
  try {
    localStorage.setItem('gm_' + key, JSON.stringify(value));
  } catch(e) {
    console.warn('[GM_setValue] Error:', e);
  }
};
window.GM_deleteValue = function(key) {
  try {
    localStorage.removeItem('gm_' + key);
  } catch(e) {
    console.warn('[GM_deleteValue] Error:', e);
  }
};
window.GM_listValues = function() {
  var keys = [];
  try {
    for (var i = 0; i < localStorage.length; i++) {
      var key = localStorage.key(i);
      if (key.indexOf('gm_') === 0) keys.push(key.substring(3));
    }
  } catch(e) {
    console.warn('[GM_listValues] Error:', e);
  }
  return keys;
};
window.GM_addStyle = function(css) {
  var style = document.createElement('style');
  style.textContent = css;
  (document.head || document.documentElement).appendChild(style);
};
window.GM_xmlhttpRequest = function(details) {
  // Use custom event to communicate with isolated context which has Tauri access
  var requestId = 'gm_xhr_' + Date.now() + '_' + Math.random().toString(36).substr(2, 9);

  // Listen for response
  var responseHandler = function(event) {
    if (event.detail && event.detail.requestId === requestId) {
      window.removeEventListener('gm_xhr_response', responseHandler);
      if (event.detail.error) {
        console.error('[GM_xmlhttpRequest] Error:', event.detail.error);
        if (details.onerror) details.onerror(event.detail.error);
      } else if (details.onload) {
        details.onload({
          responseText: event.detail.responseText,
          status: event.detail.status,
          statusText: event.detail.statusText,
          responseHeaders: event.detail.responseHeaders
        });
      }
    }
  };
  window.addEventListener('gm_xhr_response', responseHandler);

  // Send request to isolated context
  window.dispatchEvent(new CustomEvent('gm_xhr_request', {
    detail: {
      requestId: requestId,
      url: details.url,
      method: details.method || 'GET',
      headers: details.headers || null,
      data: details.data || null
    }
  }));
};
// GM_openInTab - opens URL in default browser
window.GM_openInTab = function(url, options) {
  window.dispatchEvent(new CustomEvent('gm_open_external', { detail: { url: url } }));
};
// Also create local references for scripts that expect them as globals
var unsafeWindow = window.unsafeWindow;
var GM_info = window.GM_info;
var GM_getValue = window.GM_getValue;
var GM_setValue = window.GM_setValue;
var GM_deleteValue = window.GM_deleteValue;
var GM_listValues = window.GM_listValues;
var GM_addStyle = window.GM_addStyle;
var GM_xmlhttpRequest = window.GM_xmlhttpRequest;
var GM_openInTab = window.GM_openInTab;
console.log('[GeoGuessr Desktop] Tampermonkey API compatibility loaded');
"#;
    let api_base64 = BASE64.encode(tampermonkey_api.as_bytes());
    // Inject Tampermonkey API into page's main world
    combined.push_str(&format!("    injectIntoPage(decodeBase64('{}'), 'tampermonkey-api');\n\n", api_base64));

    // Inject custom titlebar with settings panel
    let titlebar_code = format!(r#"(function() {{
  // Create titlebar container
  var titlebar = document.createElement('div');
  titlebar.id = 'gg-desktop-titlebar';
  titlebar.setAttribute('data-tauri-drag-region', '');
  titlebar.innerHTML = `
    <div class="gg-titlebar-title" data-tauri-drag-region>GeoGuessr Desktop</div>
    <div class="gg-titlebar-controls">
      <button id="gg-settings-btn" title="Settings">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="3"></circle>
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path>
        </svg>
      </button>
      <button id="gg-minimize-btn" title="Minimize">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
      </button>
      <button id="gg-maximize-btn" title="Maximize">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect>
        </svg>
      </button>
      <button id="gg-close-btn" title="Close" class="gg-close">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="18" y1="6" x2="6" y2="18"></line>
          <line x1="6" y1="6" x2="18" y2="18"></line>
        </svg>
      </button>
    </div>
  `;

  // Create settings panel
  var settingsPanel = document.createElement('div');
  settingsPanel.id = 'gg-settings-panel';
  settingsPanel.style.display = 'none';
  settingsPanel.innerHTML = `
    <div class="gg-settings-header">Scripts</div>
    <div class="gg-settings-disclaimer">Scripts run at your own risk. We are not responsible for any issues caused by third-party scripts.</div>
    <div id="gg-scripts-list"></div>
    <div class="gg-settings-add">
      <input type="text" id="gg-add-url" placeholder="Script URL (https://...)" />
      <button id="gg-add-btn">Add</button>
    </div>
    <div class="gg-settings-actions">
      <button id="gg-apply-btn" disabled>Apply &amp; Reload</button>
    </div>
    <div id="gg-settings-status"></div>
  `;

  // Add styles
  var style = document.createElement('style');
  style.textContent = `
    #gg-desktop-titlebar {{
      position: fixed;
      top: 0;
      left: 0;
      right: 0;
      height: 36px;
      background: linear-gradient(180deg, #1a1a2e 0%, #16162a 100%);
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 0 8px;
      z-index: 999999;
      user-select: none;
      -webkit-app-region: drag;
      border-bottom: 1px solid #2a2a4a;
    }}
    .gg-titlebar-title {{
      color: #e0e0e0;
      font-size: 13px;
      font-weight: 500;
      padding-left: 8px;
      -webkit-app-region: drag;
    }}
    .gg-titlebar-controls {{
      display: flex;
      gap: 2px;
      -webkit-app-region: no-drag;
    }}
    .gg-titlebar-controls button {{
      width: 36px;
      height: 28px;
      border: none;
      background: transparent;
      color: #b0b0b0;
      cursor: pointer;
      display: flex;
      align-items: center;
      justify-content: center;
      border-radius: 4px;
      transition: background 0.15s, color 0.15s;
    }}
    .gg-titlebar-controls button:hover {{
      background: rgba(255,255,255,0.1);
      color: #fff;
    }}
    .gg-titlebar-controls button.gg-close:hover {{
      background: #e81123;
      color: #fff;
    }}
    #gg-settings-panel {{
      position: fixed;
      top: 40px;
      right: 8px;
      width: 320px;
      max-height: calc(100vh - 60px);
      background: #1a1a2e;
      border: 1px solid #2a2a4a;
      border-radius: 8px;
      z-index: 999998;
      box-shadow: 0 8px 32px rgba(0,0,0,0.4);
      overflow: hidden;
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    }}
    .gg-settings-header {{
      padding: 12px 16px;
      font-size: 14px;
      font-weight: 600;
      color: #fff;
      background: #252542;
      border-bottom: 1px solid #2a2a4a;
    }}
    .gg-settings-disclaimer {{
      padding: 8px 16px;
      font-size: 11px;
      color: #b0a000;
      background: rgba(176, 160, 0, 0.1);
      border-bottom: 1px solid #2a2a4a;
      text-align: center;
    }}
    #gg-scripts-list {{
      max-height: 300px;
      overflow-y: auto;
    }}
    .gg-script-item {{
      display: flex;
      align-items: center;
      padding: 10px 16px;
      border-bottom: 1px solid #2a2a4a;
      gap: 12px;
    }}
    .gg-script-item:last-child {{
      border-bottom: none;
    }}
    .gg-script-toggle {{
      position: relative;
      width: 40px;
      height: 22px;
      background: #3a3a5a;
      border-radius: 11px;
      cursor: pointer;
      transition: background 0.2s;
      flex-shrink: 0;
    }}
    .gg-script-toggle.enabled {{
      background: #6c5ce7;
    }}
    .gg-script-toggle::after {{
      content: '';
      position: absolute;
      top: 2px;
      left: 2px;
      width: 18px;
      height: 18px;
      background: #fff;
      border-radius: 50%;
      transition: transform 0.2s;
    }}
    .gg-script-toggle.enabled::after {{
      transform: translateX(18px);
    }}
    .gg-script-info {{
      flex: 1;
      min-width: 0;
    }}
    .gg-script-name {{
      color: #e0e0e0;
      font-size: 13px;
      font-weight: 500;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }}
    .gg-script-meta {{
      color: #808080;
      font-size: 11px;
      margin-top: 2px;
    }}
    .gg-script-refresh {{
      padding: 4px 8px;
      background: transparent;
      border: 1px solid #3a3a5a;
      border-radius: 4px;
      color: #b0b0b0;
      cursor: pointer;
      font-size: 12px;
      transition: all 0.15s;
    }}
    .gg-script-refresh:hover {{
      background: #2a2a4a;
      color: #fff;
    }}
    .gg-script-delete {{
      padding: 4px 8px;
      background: transparent;
      border: 1px solid #5a3a3a;
      border-radius: 4px;
      color: #e05050;
      cursor: pointer;
      font-size: 12px;
      transition: all 0.15s;
    }}
    .gg-script-delete:hover {{
      background: #4a2a2a;
      color: #ff6060;
    }}
    .gg-settings-add {{
      display: flex;
      padding: 12px 16px;
      gap: 8px;
      border-top: 1px solid #2a2a4a;
    }}
    .gg-settings-add input {{
      flex: 1;
      padding: 8px 12px;
      background: #252542;
      border: 1px solid #3a3a5a;
      border-radius: 4px;
      color: #e0e0e0;
      font-size: 12px;
    }}
    .gg-settings-add input::placeholder {{
      color: #606080;
    }}
    .gg-settings-add input:focus {{
      outline: none;
      border-color: #6c5ce7;
    }}
    .gg-settings-add button {{
      padding: 8px 16px;
      background: #6c5ce7;
      border: none;
      border-radius: 4px;
      color: #fff;
      font-size: 12px;
      font-weight: 500;
      cursor: pointer;
      transition: background 0.15s;
    }}
    .gg-settings-add button:hover {{
      background: #5b4cdb;
    }}
    .gg-settings-actions {{
      padding: 12px 16px;
      border-top: 1px solid #2a2a4a;
    }}
    #gg-apply-btn {{
      width: 100%;
      padding: 10px 16px;
      background: #00b894;
      border: none;
      border-radius: 4px;
      color: #fff;
      font-size: 13px;
      font-weight: 500;
      cursor: pointer;
      transition: background 0.15s;
    }}
    #gg-apply-btn:hover {{
      background: #00a383;
    }}
    #gg-apply-btn:disabled,
    #gg-apply-btn.disabled {{
      background: #3a3a5a;
      color: #606080;
      cursor: not-allowed;
    }}
    #gg-apply-btn:disabled:hover,
    #gg-apply-btn.disabled:hover {{
      background: #3a3a5a;
    }}
    #gg-settings-status {{
      padding: 0 16px 12px;
      font-size: 12px;
      color: #808080;
      text-align: center;
    }}
    #gg-settings-status.error {{
      color: #e05050;
    }}
    #gg-settings-status.success {{
      color: #00b894;
    }}
    /* Adjust page content to account for titlebar */
    html {{
      margin: 0 !important;
      padding: 0 !important;
    }}
    body {{
      margin: 0 !important;
      padding-top: 36px !important;
      box-sizing: border-box !important;
      min-height: 100vh !important;
    }}
    /* Hide scrollbar but allow scrolling */
    html::-webkit-scrollbar,
    body::-webkit-scrollbar {{
      display: none !important;
      width: 0 !important;
    }}
    html, body {{
      scrollbar-width: none !important;  /* Firefox */
      -ms-overflow-style: none !important;  /* IE/Edge */
    }}
    /* Fix GeoGuessr sidebar overlapping titlebar */
    [class*="version4_sidebar"] {{
      top: 36px !important;
      min-height: calc(100% - 36px) !important;
    }}
    /* But keep scrollbar for settings panel script list */
    #gg-scripts-list::-webkit-scrollbar {{
      display: block !important;
      width: 6px;
    }}
    #gg-scripts-list::-webkit-scrollbar-track {{
      background: transparent;
    }}
    #gg-scripts-list::-webkit-scrollbar-thumb {{
      background: #3a3a5a;
      border-radius: 3px;
    }}
    .gg-no-scripts {{
      padding: 20px 16px;
      text-align: center;
      color: #606080;
      font-size: 13px;
    }}
  `;

  // Wait for body to exist before appending
  function appendElements() {{
    if (document.body) {{
      document.body.appendChild(titlebar);
      document.body.appendChild(settingsPanel);
      document.head.appendChild(style);
      initTitlebar();
    }} else {{
      requestAnimationFrame(appendElements);
    }}
  }}
  appendElements();

  // Initialize titlebar functionality
  function initTitlebar() {{
    var scriptsData = {scripts_json};
    var pendingChanges = {{}};
    var hasChanges = false;

    // Update Apply button state
    function updateApplyButton() {{
      var btn = document.getElementById('gg-apply-btn');
      if (btn) {{
        btn.disabled = !hasChanges;
        if (hasChanges) {{
          btn.classList.remove('disabled');
        }} else {{
          btn.classList.add('disabled');
        }}
      }}
    }}

    // Render scripts list
    function renderScripts() {{
      var list = document.getElementById('gg-scripts-list');
      if (!list) return;

      if (scriptsData.length === 0) {{
        list.innerHTML = '<div class="gg-no-scripts">No scripts installed.<br>Add a script URL below to get started.</div>';
        return;
      }}

      list.innerHTML = scriptsData.map(function(script) {{
        var isEnabled = pendingChanges[script.id] !== undefined ? pendingChanges[script.id] : script.enabled;
        return `
          <div class="gg-script-item" data-id="${{script.id}}">
            <div class="gg-script-toggle ${{isEnabled ? 'enabled' : ''}}" data-id="${{script.id}}"></div>
            <div class="gg-script-info">
              <div class="gg-script-name">${{script.name}}</div>
              <div class="gg-script-meta">${{script.version || 'No version'}}${{script.author ? ' by ' + script.author : ''}}</div>
            </div>
            ${{script.url ? '<button class="gg-script-refresh" data-id="' + script.id + '">↻</button>' : ''}}
            <button class="gg-script-delete" data-id="${{script.id}}">×</button>
          </div>
        `;
      }}).join('');

      // Add toggle handlers
      list.querySelectorAll('.gg-script-toggle').forEach(function(toggle) {{
        toggle.addEventListener('click', function() {{
          var id = this.dataset.id;
          var currentState = this.classList.contains('enabled');
          this.classList.toggle('enabled');
          pendingChanges[id] = !currentState;
          hasChanges = true;
          updateApplyButton();
        }});
      }});

      // Add refresh handlers
      list.querySelectorAll('.gg-script-refresh').forEach(function(btn) {{
        btn.addEventListener('click', function() {{
          var id = this.dataset.id;
          var statusEl = document.getElementById('gg-settings-status');
          statusEl.textContent = 'Refreshing script...';
          statusEl.className = '';

          var requestId = 'req_' + Date.now();
          var handler = function(e) {{
            if (e.data && e.data.type === 'gg_invoke_response' && e.data.requestId === requestId) {{
              window.removeEventListener('message', handler);
              if (e.data.error) {{
                statusEl.textContent = 'Error: ' + e.data.error;
                statusEl.className = 'error';
              }} else {{
                var idx = scriptsData.findIndex(function(s) {{ return s.id === id; }});
                if (idx !== -1) scriptsData[idx] = e.data.result;
                renderScripts();
                hasChanges = true;
                updateApplyButton();
                statusEl.textContent = 'Script refreshed! Click Apply & Reload to use.';
                statusEl.className = 'success';
              }}
            }}
          }};
          window.addEventListener('message', handler);
          window.postMessage({{ type: 'gg_invoke', requestId: requestId, command: 'refresh_script', args: {{ id: id }} }}, '*');
        }});
      }});

      // Add delete handlers
      list.querySelectorAll('.gg-script-delete').forEach(function(btn) {{
        btn.addEventListener('click', function() {{
          var id = this.dataset.id;
          var statusEl = document.getElementById('gg-settings-status');

          if (!confirm('Delete this script?')) return;

          var requestId = 'req_' + Date.now();
          var handler = function(e) {{
            if (e.data && e.data.type === 'gg_invoke_response' && e.data.requestId === requestId) {{
              window.removeEventListener('message', handler);
              if (e.data.error) {{
                statusEl.textContent = 'Error: ' + e.data.error;
                statusEl.className = 'error';
              }} else {{
                scriptsData = scriptsData.filter(function(s) {{ return s.id !== id; }});
                delete pendingChanges[id];
                renderScripts();
                hasChanges = true;
                updateApplyButton();
                statusEl.textContent = 'Script deleted. Click Apply & Reload to update.';
                statusEl.className = 'success';
              }}
            }}
          }};
          window.addEventListener('message', handler);
          window.postMessage({{ type: 'gg_invoke', requestId: requestId, command: 'delete_script', args: {{ id: id }} }}, '*');
        }});
      }});
    }}

    renderScripts();

    // Settings panel toggle
    document.getElementById('gg-settings-btn').addEventListener('click', function(e) {{
      e.stopPropagation();
      var panel = document.getElementById('gg-settings-panel');
      panel.style.display = panel.style.display === 'none' ? 'block' : 'none';
    }});

    // Close settings when clicking outside
    document.addEventListener('click', function(e) {{
      var panel = document.getElementById('gg-settings-panel');
      var settingsBtn = document.getElementById('gg-settings-btn');
      if (panel.style.display !== 'none' && !panel.contains(e.target) && e.target !== settingsBtn) {{
        panel.style.display = 'none';
      }}
    }});

    // Window controls - use postMessage to communicate with isolated context
    document.getElementById('gg-minimize-btn').addEventListener('click', function() {{
      window.postMessage({{ type: 'gg_window_control', action: 'minimize' }}, '*');
    }});

    document.getElementById('gg-maximize-btn').addEventListener('click', function() {{
      window.postMessage({{ type: 'gg_window_control', action: 'maximize' }}, '*');
    }});

    document.getElementById('gg-close-btn').addEventListener('click', function() {{
      window.postMessage({{ type: 'gg_window_control', action: 'close' }}, '*');
    }});

    // Add script button
    document.getElementById('gg-add-btn').addEventListener('click', function() {{
      var input = document.getElementById('gg-add-url');
      var url = input.value.trim();
      var statusEl = document.getElementById('gg-settings-status');

      if (!url) {{
        statusEl.textContent = 'Please enter a script URL';
        statusEl.className = 'error';
        return;
      }}

      if (!url.startsWith('https://')) {{
        statusEl.textContent = 'Only HTTPS URLs are supported';
        statusEl.className = 'error';
        return;
      }}

      statusEl.textContent = 'Adding script...';
      statusEl.className = '';

      var requestId = 'req_' + Date.now();
      var handler = function(e) {{
        if (e.data && e.data.type === 'gg_invoke_response' && e.data.requestId === requestId) {{
          window.removeEventListener('message', handler);
          if (e.data.error) {{
            statusEl.textContent = 'Error: ' + e.data.error;
            statusEl.className = 'error';
          }} else {{
            scriptsData.push(e.data.result);
            renderScripts();
            hasChanges = true;
            updateApplyButton();
            input.value = '';
            statusEl.textContent = 'Script added! Click Apply & Reload to activate.';
            statusEl.className = 'success';
          }}
        }}
      }};
      window.addEventListener('message', handler);
      window.postMessage({{ type: 'gg_invoke', requestId: requestId, command: 'add_script_from_url', args: {{ url: url }} }}, '*');
    }});

    // Apply & Reload button
    document.getElementById('gg-apply-btn').addEventListener('click', function() {{
      var statusEl = document.getElementById('gg-settings-status');
      statusEl.textContent = 'Applying changes...';
      statusEl.className = '';

      // Apply pending toggle changes sequentially
      var ids = Object.keys(pendingChanges);
      var index = 0;
      var hasError = false;

      function reloadWindow() {{
        // Use reload_scripts command to close and reopen window with fresh init script
        var requestId = 'req_reload_' + Date.now();
        window.postMessage({{ type: 'gg_invoke', requestId: requestId, command: 'reload_scripts', args: {{}} }}, '*');
      }}

      function applyNext() {{
        if (hasError || index >= ids.length) {{
          if (!hasError) {{
            // All done, reload window properly
            reloadWindow();
          }}
          return;
        }}

        var id = ids[index];
        var enabled = pendingChanges[id];
        var requestId = 'req_' + Date.now() + '_' + index;

        var handler = function(e) {{
          if (e.data && e.data.type === 'gg_invoke_response' && e.data.requestId === requestId) {{
            window.removeEventListener('message', handler);
            if (e.data.error) {{
              statusEl.textContent = 'Error: ' + e.data.error;
              statusEl.className = 'error';
              hasError = true;
            }} else {{
              index++;
              applyNext();
            }}
          }}
        }};

        window.addEventListener('message', handler);
        window.postMessage({{ type: 'gg_invoke', requestId: requestId, command: 'toggle_script', args: {{ id: id, enabled: enabled }} }}, '*');
      }}

      if (ids.length === 0) {{
        // No changes, just reload window
        reloadWindow();
      }} else {{
        applyNext();
      }}
    }});

    // Initialize button state (disabled by default)
    updateApplyButton();
  }}
}})();"#, scripts_json = all_scripts_json);

    let titlebar_base64 = BASE64.encode(titlebar_code.as_bytes());
    combined.push_str(&format!("    injectIntoPage(decodeBase64('{}'), 'custom-titlebar');\n\n", titlebar_base64));

    // Discord Rich Presence integration with GeoGuessr Event Framework
    let discord_presence_code = r#"(function() {
  // Prevent re-initialization on SPA navigation
  if (window.__ggDiscordPresenceInitialized) {
    return;
  }
  window.__ggDiscordPresenceInitialized = true;

  console.log('[Discord Presence] Initializing...');

  var currentMapName = null;
  var gefLoaded = false;
  var inGame = false; // True when GEF is actively tracking a game

  // Update Discord presence
  function updatePresence(details, state) {
    window.postMessage({
      type: 'gg_invoke',
      requestId: 'discord_' + Date.now(),
      command: 'discord_update_presence',
      args: {
        details: details || 'GeoGuessr',
        presence_state: state || null,
        start_timestamp: null
      }
    }, '*');
  }

  // Connect to Discord
  function connectDiscord() {
    console.log('[Discord Presence] Connecting to Discord...');
    window.postMessage({
      type: 'gg_invoke',
      requestId: 'discord_connect_' + Date.now(),
      command: 'discord_connect',
      args: {}
    }, '*');
  }

  // Initialize GEF event listeners
  function initGefListeners() {
    // GEF is exposed as window.GeoGuessrEventFramework
    var gef = window.GeoGuessrEventFramework;
    if (!gef || !gef.events) {
      console.log('[Discord Presence] GEF not loaded yet, waiting...');
      return false;
    }

    console.log('[Discord Presence] GEF loaded, setting up event listeners');
    gefLoaded = true;

    // Game start - triggered at round 1
    gef.events.addEventListener('game_start', function(event) {
      console.log('[Discord Presence] Game started:', event.detail);
      var state = event.detail;
      inGame = true;

      currentMapName = state.map && state.map.name ? state.map.name : null;
      var details = currentMapName ? currentMapName : 'Playing';
      updatePresence(details, 'Round 1');
    });

    // Round start
    gef.events.addEventListener('round_start', function(event) {
      console.log('[Discord Presence] Round started:', event.detail);
      var state = event.detail;

      currentMapName = state.map && state.map.name ? state.map.name : null;
      var details = currentMapName ? currentMapName : 'Playing';
      var presenceState = state.current_round ? 'Round ' + state.current_round : null;
      updatePresence(details, presenceState);
    });

    // Round end
    gef.events.addEventListener('round_end', function(event) {
      console.log('[Discord Presence] Round ended:', event.detail);
      var state = event.detail;

      var details = currentMapName ? currentMapName : 'Playing';
      var presenceState = state.total_score ?
        'Score: ' + state.total_score.amount + ' pts' :
        'Round ' + state.current_round + ' complete';
      updatePresence(details, presenceState);
    });

    // Game end
    gef.events.addEventListener('game_end', function(event) {
      console.log('[Discord Presence] Game ended:', event.detail);
      inGame = false;
      updatePresence('Menus', null);
    });

    return true;
  }

  // Check if GEF is already loaded
  function isGefLoaded() {
    return typeof window.GeoGuessrEventFramework !== 'undefined' &&
           window.GeoGuessrEventFramework.events;
  }

  // Load GEF if not already loaded (with delay to let userscript deps load first)
  function loadGefIfNeeded() {
    // Wait 2 seconds for userscript dependencies to load GEF first
    console.log('[Discord Presence] Waiting for dependencies to load...');
    setTimeout(function() {
      if (isGefLoaded()) {
        console.log('[Discord Presence] GEF already loaded by dependencies');
        if (initGefListeners()) {
          console.log('[Discord Presence] GEF event listeners set up successfully');
        }
        return;
      }

      // GEF not loaded by dependencies, load it ourselves
      console.log('[Discord Presence] Loading GEF ourselves...');
      var script = document.createElement('script');
      script.src = 'https://miraclewhips.dev/geoguessr-event-framework/geoguessr-event-framework.min.js';
      script.onload = function() {
        console.log('[Discord Presence] GEF script loaded');
        waitForGef();
      };
      script.onerror = function() {
        console.error('[Discord Presence] Failed to load GEF');
      };
      document.head.appendChild(script);
    }, 2000);
  }

  // Wait for GEF to initialize
  function waitForGef() {
    var retries = 0;
    var maxRetries = 20; // 10 seconds max
    var interval = setInterval(function() {
      if (initGefListeners()) {
        clearInterval(interval);
        console.log('[Discord Presence] GEF event listeners set up successfully');
      } else if (retries >= maxRetries) {
        clearInterval(interval);
        console.log('[Discord Presence] GEF not available');
      }
      retries++;
    }, 500);
  }

  // Check if user left a game (for early exit detection)
  function isGameUrl() {
    var path = window.location.pathname;
    return path.includes('/game/') || path.includes('/duels') ||
           path.includes('/battle-royale') || path.includes('/challenge');
  }

  function watchForGameExit() {
    setInterval(function() {
      if (inGame && !isGameUrl()) {
        console.log('[Discord Presence] Left game early, returning to Menus');
        inGame = false;
        updatePresence('Menus', null);
      }
    }, 1000);
  }

  // Start everything when DOM is ready
  function init() {
    connectDiscord();
    loadGefIfNeeded();
    watchForGameExit();
    // Delay initial presence to let discord_connect complete
    setTimeout(function() {
      updatePresence('Menus', null);
    }, 1000);
  }

  // Start when body exists
  if (document.body) {
    init();
  } else {
    document.addEventListener('DOMContentLoaded', init);
  }
})();"#;

    let discord_base64 = BASE64.encode(discord_presence_code.as_bytes());
    combined.push_str(&format!("    injectIntoPage(decodeBase64('{}'), 'discord-presence');\n\n", discord_base64));

    // Collect all unique dependencies across all enabled scripts
    let mut all_requires: Vec<String> = Vec::new();
    let mut seen_requires: HashSet<String> = HashSet::new();
    for script in &enabled_scripts {
        for req_url in &script.requires {
            if seen_requires.insert(req_url.clone()) {
                all_requires.push(req_url.clone());
            }
        }
    }

    // Inject dependencies into page's main world - this is critical for fetch interceptors
    // They need to wrap fetch BEFORE the page makes any requests
    if !all_requires.is_empty() {
        combined.push_str("    // === Injecting userscript dependencies ===\n");
        for (dep_index, req_url) in all_requires.iter().enumerate() {
            if let Some(dep) = dependencies.get(req_url) {
                combined.push_str(&format!("    console.log('[GeoGuessr Desktop] Loading dependency: {}');\n", req_url));
                // Use base64 encoding to avoid escaping issues
                let dep_base64 = BASE64.encode(dep.code.as_bytes());
                // Inject into page's main world
                combined.push_str(&format!("    injectIntoPage(decodeBase64('{}'), 'dependency-{}');\n",
                    dep_base64, dep_index));
            } else {
                combined.push_str(&format!("    console.warn('[GeoGuessr Desktop] Missing dependency: {}');\n", req_url));
            }
        }
        combined.push_str("    console.log('[GeoGuessr Desktop] Dependencies loaded');\n\n");
    }

    // Inject userscripts into page's main world
    combined.push_str("    // === Injecting userscripts ===\n");
    for script in enabled_scripts {
        combined.push_str(&format!("    console.log('[GeoGuessr Desktop] Queuing script: {}');\n", script.name));

        // Wrap the script to run on load, then encode as base64
        let wrapped_script = format!(r#"(function() {{
  var runScript = function() {{
    try {{
      console.log('[GeoGuessr Desktop] Executing script: {}');
{}
      console.log('[GeoGuessr Desktop] Script completed: {}');
    }} catch(e) {{
      console.error('[GeoGuessr Desktop] Error in script {}: ', e);
    }}
  }};
  if (document.readyState === 'complete') {{
    runScript();
  }} else {{
    window.addEventListener('load', runScript);
  }}
}})();"#, script.name, script.code, script.name, script.name);

        let script_base64 = BASE64.encode(wrapped_script.as_bytes());
        // Inject into page's main world
        combined.push_str(&format!("    injectIntoPage(decodeBase64('{}'), '{}');\n\n",
            script_base64, script.name));
    }

    // Close the waitForDocumentElement callback
    combined.push_str("  });\n");

    // Set up GM_xmlhttpRequest bridge in isolated context (has Tauri access)
    combined.push_str("\n  // GM_xmlhttpRequest bridge - listens for requests from page and uses Tauri\n");
    combined.push_str("  window.addEventListener('gm_xhr_request', function(event) {\n");
    combined.push_str("    var detail = event.detail;\n");
    combined.push_str("    if (!detail || !detail.requestId || !detail.url) return;\n");
    combined.push_str("    \n");
    combined.push_str("    var request = {\n");
    combined.push_str("      url: detail.url,\n");
    combined.push_str("      method: detail.method || 'GET',\n");
    combined.push_str("      headers: detail.headers,\n");
    combined.push_str("      data: detail.data\n");
    combined.push_str("    };\n");
    combined.push_str("    \n");
    combined.push_str("    if (window.__TAURI__ && window.__TAURI__.core) {\n");
    combined.push_str("      window.__TAURI__.core.invoke('gm_xhr', { request: request })\n");
    combined.push_str("        .then(function(response) {\n");
    combined.push_str("          window.dispatchEvent(new CustomEvent('gm_xhr_response', {\n");
    combined.push_str("            detail: {\n");
    combined.push_str("              requestId: detail.requestId,\n");
    combined.push_str("              responseText: response.response_text,\n");
    combined.push_str("              status: response.status,\n");
    combined.push_str("              statusText: response.status_text,\n");
    combined.push_str("              responseHeaders: response.response_headers\n");
    combined.push_str("            }\n");
    combined.push_str("          }));\n");
    combined.push_str("        })\n");
    combined.push_str("        .catch(function(error) {\n");
    combined.push_str("          window.dispatchEvent(new CustomEvent('gm_xhr_response', {\n");
    combined.push_str("            detail: {\n");
    combined.push_str("              requestId: detail.requestId,\n");
    combined.push_str("              error: error.toString()\n");
    combined.push_str("            }\n");
    combined.push_str("          }));\n");
    combined.push_str("        });\n");
    combined.push_str("    } else {\n");
    combined.push_str("      console.error('[GM_xmlhttpRequest Bridge] Tauri API not available');\n");
    combined.push_str("      window.dispatchEvent(new CustomEvent('gm_xhr_response', {\n");
    combined.push_str("        detail: {\n");
    combined.push_str("          requestId: detail.requestId,\n");
    combined.push_str("          error: 'Tauri API not available'\n");
    combined.push_str("        }\n");
    combined.push_str("      }));\n");
    combined.push_str("    }\n");
    combined.push_str("  });\n");
    combined.push_str("  console.log('[GeoGuessr Desktop] GM_xmlhttpRequest bridge initialized');\n\n");

    // External URL opener bridge
    combined.push_str("  // External URL opener bridge\n");
    combined.push_str("  window.addEventListener('gm_open_external', function(event) {\n");
    combined.push_str("    var url = event.detail && event.detail.url;\n");
    combined.push_str("    if (!url) return;\n");
    combined.push_str("    if (window.__TAURI__ && window.__TAURI__.core) {\n");
    combined.push_str("      window.__TAURI__.core.invoke('open_external_url', { url: url })\n");
    combined.push_str("        .catch(function(e) { console.error('[Open External] Error:', e); });\n");
    combined.push_str("    }\n");
    combined.push_str("  });\n\n");

    // Intercept external link clicks
    combined.push_str("  // Intercept external link clicks and open in default browser\n");
    combined.push_str("  document.addEventListener('click', function(e) {\n");
    combined.push_str("    var target = e.target;\n");
    combined.push_str("    while (target && target.tagName !== 'A') {\n");
    combined.push_str("      target = target.parentElement;\n");
    combined.push_str("    }\n");
    combined.push_str("    if (!target || !target.href) return;\n");
    combined.push_str("    \n");
    combined.push_str("    var url = target.href;\n");
    combined.push_str("    var isExternal = !url.includes('geoguessr.com');\n");
    combined.push_str("    if (isExternal && window.__TAURI__ && window.__TAURI__.core) {\n");
    combined.push_str("      e.preventDefault();\n");
    combined.push_str("      e.stopPropagation();\n");
    combined.push_str("      console.log('[GeoGuessr Desktop] Opening external URL:', url);\n");
    combined.push_str("      window.__TAURI__.core.invoke('open_external_url', { url: url })\n");
    combined.push_str("        .catch(function(e) { console.error('[Open External] Error:', e); });\n");
    combined.push_str("    }\n");
    combined.push_str("  }, true);\n");
    combined.push_str("  console.log('[GeoGuessr Desktop] External link handler initialized');\n\n");

    // Message bridge - handles postMessage from page's main world
    combined.push_str("  // Message bridge - handles postMessage from page\n");
    combined.push_str("  window.addEventListener('message', function(event) {\n");
    combined.push_str("    var data = event.data;\n");
    combined.push_str("    if (!data || !data.type) return;\n");
    combined.push_str("    \n");
    combined.push_str("    // Window control\n");
    combined.push_str("    if (data.type === 'gg_window_control') {\n");
    combined.push_str("      var action = data.action;\n");
    combined.push_str("      console.log('[GeoGuessr Desktop] Window control:', action);\n");
    combined.push_str("      if (window.__TAURI__ && window.__TAURI__.window) {\n");
    combined.push_str("        var win = window.__TAURI__.window.getCurrentWindow();\n");
    combined.push_str("        if (action === 'minimize') win.minimize();\n");
    combined.push_str("        else if (action === 'maximize') win.toggleMaximize();\n");
    combined.push_str("        else if (action === 'close') win.close();\n");
    combined.push_str("      } else {\n");
    combined.push_str("        console.error('[GeoGuessr Desktop] Tauri window API not available');\n");
    combined.push_str("      }\n");
    combined.push_str("    }\n");
    combined.push_str("    \n");
    combined.push_str("    // Generic invoke\n");
    combined.push_str("    if (data.type === 'gg_invoke') {\n");
    combined.push_str("      if (!data.requestId || !data.command) return;\n");
    combined.push_str("      console.log('[GeoGuessr Desktop] Invoke:', data.command, data.args);\n");
    combined.push_str("      \n");
    combined.push_str("      if (window.__TAURI__ && window.__TAURI__.core) {\n");
    combined.push_str("        window.__TAURI__.core.invoke(data.command, data.args || {})\n");
    combined.push_str("          .then(function(result) {\n");
    combined.push_str("            window.postMessage({ type: 'gg_invoke_response', requestId: data.requestId, result: result }, '*');\n");
    combined.push_str("          })\n");
    combined.push_str("          .catch(function(error) {\n");
    combined.push_str("            window.postMessage({ type: 'gg_invoke_response', requestId: data.requestId, error: error.toString() }, '*');\n");
    combined.push_str("          });\n");
    combined.push_str("      } else {\n");
    combined.push_str("        console.error('[GeoGuessr Desktop] Tauri core API not available');\n");
    combined.push_str("        window.postMessage({ type: 'gg_invoke_response', requestId: data.requestId, error: 'Tauri API not available' }, '*');\n");
    combined.push_str("      }\n");
    combined.push_str("    }\n");
    combined.push_str("  });\n");
    combined.push_str("  console.log('[GeoGuessr Desktop] Message bridge initialized');\n");

    // Close main IIFE
    combined.push_str("})();\n");

    combined
}

#[tauri::command]
async fn reload_scripts(app: tauri::AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Set reloading flag to prevent app exit
    set_reloading(true);

    // Get fresh initialization script
    let init_script = get_initialization_script(&state);

    // Close old window if it exists
    if let Some(window) = app.get_webview_window("geoguessr") {
        let _ = window.close();
    }

    // Small delay to allow window to close
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Create new window with updated scripts
    let result = WebviewWindowBuilder::new(
        &app,
        "geoguessr",
        WebviewUrl::External("https://www.geoguessr.com/".parse().unwrap())
    )
        .title("GeoGuessr Desktop")
        .inner_size(1400.0, 900.0)
        .resizable(true)
        .decorations(false)
        .initialization_script(&init_script)
        .on_navigation(move |url| {
            url.host_str() == Some("www.geoguessr.com") ||
            url.host_str() == Some("geoguessr.com")
        })
        .build()
        .map_err(|e| format!("Failed to create window: {}", e));

    // Reset reloading flag
    set_reloading(false);

    result.map(|_| ())
}

#[tauri::command]
async fn close_geoguessr(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("geoguessr") {
        window.close().map_err(|e| format!("Failed to close window: {}", e))?;
    }
    Ok(())
}

// Open URL in default browser
#[tauri::command]
async fn open_external_url(url: String) -> Result<(), String> {
    open::that(&url).map_err(|e| format!("Failed to open URL: {}", e))
}

// GM_xmlhttpRequest backend - bypasses CORS by making request from Rust
#[derive(Debug, Deserialize)]
struct GmXhrRequest {
    url: String,
    method: Option<String>,
    headers: Option<HashMap<String, String>>,
    data: Option<String>,
}

#[derive(Debug, Serialize)]
struct GmXhrResponse {
    response_text: String,
    status: u16,
    status_text: String,
    response_headers: String,
}

#[tauri::command]
async fn gm_xhr(request: GmXhrRequest) -> Result<GmXhrResponse, String> {
    let client = reqwest::Client::new();

    let method = request.method.unwrap_or_else(|| "GET".to_string());
    let mut req_builder = match method.to_uppercase().as_str() {
        "POST" => client.post(&request.url),
        "PUT" => client.put(&request.url),
        "DELETE" => client.delete(&request.url),
        "HEAD" => client.head(&request.url),
        "PATCH" => client.patch(&request.url),
        _ => client.get(&request.url),
    };

    // Add custom headers
    if let Some(headers) = request.headers {
        for (key, value) in headers {
            req_builder = req_builder.header(&key, &value);
        }
    }

    // Add body data for POST/PUT/PATCH
    if let Some(data) = request.data {
        req_builder = req_builder.body(data);
    }

    let response = req_builder.send().await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status().as_u16();
    let status_text = response.status().canonical_reason().unwrap_or("").to_string();

    // Collect response headers
    let response_headers: Vec<String> = response.headers()
        .iter()
        .map(|(k, v)| format!("{}: {}", k.as_str(), v.to_str().unwrap_or("")))
        .collect();

    let response_text = response.text().await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    Ok(GmXhrResponse {
        response_text,
        status,
        status_text,
        response_headers: response_headers.join("\r\n"),
    })
}

use std::sync::atomic::{AtomicBool, Ordering};

static RELOADING: AtomicBool = AtomicBool::new(false);

pub fn set_reloading(value: bool) {
    RELOADING.store(value, Ordering::SeqCst);
}

pub fn is_reloading() -> bool {
    RELOADING.load(Ordering::SeqCst)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            get_scripts,
            add_script_from_url,
            toggle_script,
            delete_script,
            reorder_script,
            refresh_script,
            auto_update_scripts,
            get_data_dir,
            open_geoguessr,
            reload_scripts,
            close_geoguessr,
            gm_xhr,
            open_external_url,
            discord_connect,
            discord_update_presence,
            discord_clear_presence,
            discord_disconnect
        ])
        .setup(|app| {
            // Open GeoGuessr window on startup
            let state = app.state::<AppState>();
            let init_script = get_initialization_script(&state);

            let _window = WebviewWindowBuilder::new(
                app,
                "geoguessr",
                WebviewUrl::External("https://www.geoguessr.com/".parse().unwrap())
            )
                .title("GeoGuessr Desktop")
                .inner_size(1400.0, 900.0)
                .resizable(true)
                .decorations(false) // Custom titlebar
                .initialization_script(&init_script)
                .on_navigation(move |url| {
                    // Allow navigation to geoguessr.com domains
                    url.host_str() == Some("www.geoguessr.com") ||
                    url.host_str() == Some("geoguessr.com")
                })
                .build()?;

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                // Prevent exit if we're in the middle of reloading
                if is_reloading() {
                    api.prevent_exit();
                }
            }
        });
}
