import { invoke } from "@tauri-apps/api/core";

interface UserScript {
  id: string;
  name: string;
  code: string;
  enabled: boolean;
  order: number;
  url?: string;
  version?: string;
  description?: string;
  author?: string;
  requires?: string[];
  last_updated?: number;
  last_fetch_error?: string;
}

let scripts: UserScript[] = [];

async function loadScripts() {
  try {
    scripts = await invoke("get_scripts");
    renderScriptsList();
  } catch (e) {
    console.error("Failed to load scripts:", e);
  }
}

function renderScriptsList() {
  const scriptsList = document.getElementById("scripts-list");
  if (!scriptsList) return;

  scriptsList.innerHTML = "";

  if (scripts.length === 0) {
    const emptyMessage = document.createElement("p");
    emptyMessage.className = "empty-message";
    emptyMessage.textContent = "No scripts added yet";
    scriptsList.appendChild(emptyMessage);
    return;
  }

  // Sort scripts by order
  const sortedScripts = [...scripts].sort((a, b) => a.order - b.order);

  sortedScripts.forEach((script, index) => {
    const scriptItem = document.createElement("div");
    scriptItem.className = "script-item";

    const orderControls = document.createElement("div");
    orderControls.className = "order-controls";

    const upBtn = document.createElement("button");
    upBtn.className = "btn-order";
    upBtn.textContent = "▲";
    upBtn.disabled = index === 0;
    upBtn.addEventListener("click", () => moveScriptUp(script.id));

    const downBtn = document.createElement("button");
    downBtn.className = "btn-order";
    downBtn.textContent = "▼";
    downBtn.disabled = index === sortedScripts.length - 1;
    downBtn.addEventListener("click", () => moveScriptDown(script.id));

    orderControls.appendChild(upBtn);
    orderControls.appendChild(downBtn);

    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = script.enabled;
    checkbox.addEventListener("change", () => toggleScript(script.id, checkbox.checked));

    const nameSpan = document.createElement("span");
    nameSpan.className = "script-name";
    nameSpan.textContent = script.name;

    // Add refresh button for URL-based scripts
    if (script.url) {
      const refreshBtn = document.createElement("button");
      refreshBtn.className = "btn-refresh";
      refreshBtn.innerHTML = "↻";
      refreshBtn.title = "Refresh from URL";
      refreshBtn.addEventListener("click", async () => {
        refreshBtn.disabled = true;
        try {
          await refreshScript(script.id);
        } catch (e) {
          alert("Refresh failed: " + e);
        } finally {
          refreshBtn.disabled = false;
        }
      });
      scriptItem.appendChild(orderControls);
      scriptItem.appendChild(checkbox);
      scriptItem.appendChild(nameSpan);
      scriptItem.appendChild(refreshBtn);
    } else {
      scriptItem.appendChild(orderControls);
      scriptItem.appendChild(checkbox);
      scriptItem.appendChild(nameSpan);
    }

    // Add error indicator
    if (script.last_fetch_error) {
      const errorIcon = document.createElement("span");
      errorIcon.className = "error-icon";
      errorIcon.title = script.last_fetch_error;
      errorIcon.innerHTML = "⚠";
      scriptItem.appendChild(errorIcon);
    }

    const deleteBtn = document.createElement("button");
    deleteBtn.className = "btn-delete";
    deleteBtn.textContent = "×";
    deleteBtn.addEventListener("click", () => deleteScript(script.id));

    scriptItem.appendChild(deleteBtn);
    scriptsList.appendChild(scriptItem);
  });
}

async function toggleScript(id: string, enabled: boolean) {
  try {
    await invoke("toggle_script", { id, enabled });
    const script = scripts.find((s) => s.id === id);
    if (script) {
      script.enabled = enabled;
    }
    await reloadScripts();
  } catch (e) {
    console.error("Failed to toggle script:", e);
  }
}

async function deleteScript(id: string) {
  try {
    await invoke("delete_script", { id });
    scripts = scripts.filter((s) => s.id !== id);
    renderScriptsList();
    await reloadScripts();
  } catch (e) {
    console.error("Failed to delete script:", e);
  }
}

async function addScriptFromUrl(url: string) {
  try {
    const newScript = await invoke("add_script_from_url", { url });
    scripts.push(newScript as UserScript);
    renderScriptsList();
    await reloadScripts();
  } catch (e) {
    console.error("Failed to add script:", e);
    throw e;
  }
}

async function refreshScript(id: string) {
  try {
    const updated = await invoke("refresh_script", { id }) as UserScript;
    const index = scripts.findIndex((s) => s.id === id);
    if (index !== -1) {
      scripts[index] = updated;
    }
    renderScriptsList();
    await reloadScripts();
  } catch (e) {
    console.error("Failed to refresh script:", e);
    throw e;
  }
}

async function openGeoGuessr() {
  try {
    await invoke("open_geoguessr");
  } catch (e) {
    console.error("Failed to open GeoGuessr:", e);
  }
}

async function reloadScripts() {
  try {
    await invoke("reload_scripts");
  } catch (e) {
    console.error("Failed to reload scripts:", e);
  }
}

async function moveScriptUp(id: string) {
  const sortedScripts = [...scripts].sort((a, b) => a.order - b.order);
  const index = sortedScripts.findIndex((s) => s.id === id);

  if (index > 0) {
    const currentScript = sortedScripts[index];
    const prevScript = sortedScripts[index - 1];

    // Swap orders
    try {
      await invoke("reorder_script", { id: currentScript.id, newOrder: prevScript.order });
      await invoke("reorder_script", { id: prevScript.id, newOrder: currentScript.order });

      currentScript.order = prevScript.order;
      prevScript.order = currentScript.order;

      renderScriptsList();
      await reloadScripts();
    } catch (e) {
      console.error("Failed to reorder script:", e);
    }
  }
}

async function moveScriptDown(id: string) {
  const sortedScripts = [...scripts].sort((a, b) => a.order - b.order);
  const index = sortedScripts.findIndex((s) => s.id === id);

  if (index < sortedScripts.length - 1) {
    const currentScript = sortedScripts[index];
    const nextScript = sortedScripts[index + 1];

    // Swap orders
    try {
      await invoke("reorder_script", { id: currentScript.id, newOrder: nextScript.order });
      await invoke("reorder_script", { id: nextScript.id, newOrder: currentScript.order });

      currentScript.order = nextScript.order;
      nextScript.order = currentScript.order;

      renderScriptsList();
      await reloadScripts();
    } catch (e) {
      console.error("Failed to reorder script:", e);
    }
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  const openGeoGuessrBtn = document.getElementById("open-geoguessr-btn");
  const addScriptBtn = document.getElementById("add-script-btn");
  const modal = document.getElementById("script-modal");
  const closeModal = document.getElementById("close-modal");
  const cancelBtn = document.getElementById("cancel-btn");
  const saveScriptBtn = document.getElementById("save-script-btn");
  const scriptUrlInput = document.getElementById("script-url") as HTMLInputElement;
  const fetchError = document.getElementById("fetch-error");
  const fetchLoading = document.getElementById("fetch-loading");

  openGeoGuessrBtn?.addEventListener("click", () => {
    openGeoGuessr();
  });

  addScriptBtn?.addEventListener("click", () => {
    modal?.classList.remove("hidden");
  });

  closeModal?.addEventListener("click", () => {
    modal?.classList.add("hidden");
    scriptUrlInput.value = "";
    fetchError?.classList.add("hidden");
    fetchLoading?.classList.add("hidden");
  });

  cancelBtn?.addEventListener("click", () => {
    modal?.classList.add("hidden");
    scriptUrlInput.value = "";
    fetchError?.classList.add("hidden");
    fetchLoading?.classList.add("hidden");
  });

  saveScriptBtn?.addEventListener("click", async () => {
    const url = scriptUrlInput.value.trim();

    if (url) {
      try {
        saveScriptBtn.disabled = true;
        fetchLoading?.classList.remove("hidden");
        fetchError?.classList.add("hidden");

        await addScriptFromUrl(url);

        modal?.classList.add("hidden");
        scriptUrlInput.value = "";
        fetchLoading?.classList.add("hidden");
      } catch (e) {
        if (fetchError) {
          fetchError.textContent = String(e);
          fetchError.classList.remove("hidden");
        }
        fetchLoading?.classList.add("hidden");
      } finally {
        saveScriptBtn.disabled = false;
      }
    }
  });

  // Enable/disable save button based on URL input
  scriptUrlInput?.addEventListener("input", () => {
    const url = scriptUrlInput.value.trim();
    if (saveScriptBtn) {
      saveScriptBtn.disabled = !url || !url.startsWith("http");
    }
  });

  await loadScripts();

  // Log data directory location
  try {
    const dataDir = await invoke("get_data_dir") as string;
    console.log("Data directory:", dataDir);
  } catch (e) {
    console.error("Failed to get data directory:", e);
  }

  // Auto-update scripts on startup
  try {
    const updateCount = await invoke("auto_update_scripts") as number;
    if (updateCount > 0) {
      console.log(`Updated ${updateCount} scripts`);
      await loadScripts();
    }
  } catch (e) {
    console.error("Auto-update failed:", e);
  }
});
