<script>
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { open } from "@tauri-apps/plugin-dialog";
  import { onMount, onDestroy } from "svelte";

  // ── State ──────────────────────────────────────────────────────
  let recording = $state(false);
  let elapsed = $state(0);
  let fileSize = $state(0);
  let savedPath = $state("");
  let errorMsg = $state("");
  let loading = $state(false);
  let noPermission = $state(false);

  // Navigation: "main" | "settings" | "history" | "presets" | "preset-edit" | "storage"
  let view = $state("main");

  // Settings
  let devices = $state([]);
  let selectedDevice = $state("");
  let selectedPresetId = $state("daily");
  let outputDir = $state("");
  let defaultOutputDir = $state("");
  let presets = $state([]);

  // Storage policy
  let storagePolicy = $state({
    cleanup_enabled: false,
    retention_days: 30,
    delete_large_old_files: false,
    large_file_threshold_mb: 500,
    max_files_to_delete_per_run: 20,
  });

  // History
  let historyEntries = $state([]);

  // Preset editor
  let editingPreset = $state(null);
  let presetEditorMode = $state("create");
  let validEncoderPresets = $state([]);
  let presetError = $state("");

  // Transcription
  let transcriptionJobs = $state([]);
  let whisperModelAvailable = $state(false);
  let modelDownloading = $state(false);
  let modelDownloadPercent = $state(0);
  let showCloseWarning = $state(false);
  let showModelPrompt = $state(false);

  // Camera
  let cameraDevices = $state([]);

  // Debug camera
  let cameraOverlayOpen = $state(false);

  // SRT viewer
  let srtContent = $state("");
  let srtViewerOpen = $state(false);

  let unlisten = [];
  let transcriptionPollInterval = null;

  // ── Formatters ─────────────────────────────────────────────────
  const fmt = (s) => {
    const m = Math.floor(s / 60).toString().padStart(2, "0");
    const sec = (s % 60).toString().padStart(2, "0");
    return `${m}:${sec}`;
  };

  const fmtSize = (b) => {
    if (b < 1024) return `${b} B`;
    if (b < 1048576) return `${(b / 1024).toFixed(0)} KB`;
    if (b < 1073741824) return `${(b / 1048576).toFixed(1)} MB`;
    return `${(b / 1073741824).toFixed(2)} GB`;
  };

  const fmtDuration = (s) => {
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s/60)}m ${s%60}s`;
    return `${Math.floor(s/3600)}h ${Math.floor((s%3600)/60)}m`;
  };

  const shortDir = (p) => {
    if (!p) return "Default";
    return p.replace(/^\/Users\/[^/]+/, "~");
  };

  const fmtDate = (iso) => {
    try {
      const d = new Date(iso);
      return d.toLocaleDateString() + " " + d.toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'});
    } catch { return iso; }
  };

  const presetColor = (name) => {
    const n = name?.toLowerCase() || "";
    if (n.includes("voice")) return "#8b5cf6";
    if (n.includes("daily")) return "#3b82f6";
    if (n.includes("meeting")) return "#f59e0b";
    if (n.includes("presentation")) return "#10b981";
    // hash-based for custom presets
    let h = 0;
    for (let i = 0; i < n.length; i++) h = n.charCodeAt(i) + ((h << 5) - h);
    const hue = Math.abs(h) % 360;
    return `hsl(${hue}, 55%, 55%)`;
  };

  // ── Recording ──────────────────────────────────────────────────
  async function toggle() {
    if (loading) return;
    loading = true;
    errorMsg = "";
    try {
      if (recording) {
        await invoke("save_camera_position").catch(() => {});
        await invoke("close_camera_overlay").catch(() => {});
        savedPath = await invoke("stop_recording");
        recording = false;
        await loadHistory();
        await loadTranscriptionQueue();
      } else {
        savedPath = "";
        await invoke("start_recording", {
          audioDevice: selectedDevice || null,
          quality: selectedPresetId,
        });
        recording = true;
        elapsed = 0;
        fileSize = 0;

        // Open camera overlay if preset has it enabled
        const activePreset = presets.find((p) => p.id === selectedPresetId);
        if (activePreset?.camera_overlay_enabled) {
          await invoke("open_camera_overlay", {
            deviceId: activePreset.camera_device_id || null,
            size: activePreset.camera_overlay_size || "medium",
            shape: activePreset.camera_overlay_shape || "circle",
          }).catch((e) => console.error("Camera overlay:", e));
        }
      }
    } catch (e) {
      errorMsg = String(e);
    }
    loading = false;
  }

  // ── Settings ───────────────────────────────────────────────────
  async function loadSettings() {
    try {
      const s = await invoke("get_settings");
      selectedPresetId = s.selected_preset_id || "daily";
      outputDir = s.output_dir || "";
      selectedDevice = s.audio_device || "";
      storagePolicy = s.storage_policy || storagePolicy;
      defaultOutputDir = await invoke("get_default_output_dir");
    } catch (_) {}
  }

  async function saveSettings() {
    try {
      await invoke("save_settings", {
        settings: {
          output_dir: outputDir || null,
          selected_preset_id: selectedPresetId,
          audio_device: selectedDevice || null,
          storage_policy: storagePolicy,
        },
      });
    } catch (e) {
      console.error("Save:", e);
    }
  }

  async function pickFolder() {
    const dir = await open({ directory: true, multiple: false });
    if (dir) { outputDir = dir; await saveSettings(); }
  }

  async function resetFolder() { outputDir = ""; await saveSettings(); }

  async function grantAccess() {
    const { requestScreenRecordingPermission } = await import("tauri-plugin-macos-permissions-api");
    await requestScreenRecordingPermission();
    await invoke("open_screen_settings");
  }

  // ── Presets ────────────────────────────────────────────────────
  async function loadPresets() {
    presets = await invoke("list_presets").catch(() => []);
    validEncoderPresets = await invoke("cmd_valid_encoder_presets").catch(() => []);
  }

  // Estimate file size per hour — calibrated against real screen recordings.
  // Model: video_kbps = BPP * crf_adj * w * h * fps * fps_penalty / 1000
  // Calibrated to ±15% on Voice-first(254), Daily(164), Meeting(299), Presentation(314) MB/h.
  function estimateFileSizePerHour(preset) {
    if (!preset) return "";
    const h = preset.target_height || 720;
    const w = Math.round(h * 16 / 9);
    const fps = Math.max(1, preset.fps || 8);
    const crf = preset.crf ?? 28;
    const audioBps = preset.audio_bitrate || 48000;

    const BPP = 0.055;        // bits per pixel per frame (screen content baseline)
    const CRF_DIV = 49;       // CRF sensitivity (screen content = very gentle)
    const FPS_THRESH = 8;     // below this fps, I-frame overhead inflates size
    const FPS_EXP = 3.3;      // I-frame penalty exponent

    const crfAdj = Math.pow(2, (28 - crf) / CRF_DIV);
    const fpsMult = fps < FPS_THRESH ? Math.pow(FPS_THRESH / fps, FPS_EXP) : 1.0;
    const videoKbps = BPP * crfAdj * w * h * fps * fpsMult / 1000;
    const audioKbps = audioBps / 1000;
    const mbPerHour = ((videoKbps + audioKbps) / 8) * 3600 / 1024;

    if (mbPerHour < 1024) {
      return `~${Math.round(mbPerHour)} MB/h`;
    }
    return `~${(mbPerHour / 1024).toFixed(1)} GB/h`;
  }

  // Clamp numeric preset field to valid range
  function clampPresetField(field, min, max) {
    if (editingPreset && editingPreset[field] != null) {
      const v = Number(editingPreset[field]);
      if (v < min) editingPreset[field] = min;
      else if (v > max) editingPreset[field] = max;
    }
  }

  // Cycle to next preset
  function cyclePreset() {
    if (!presets.length) return;
    const idx = presets.findIndex(p => p.id === selectedPresetId);
    const next = (idx + 1) % presets.length;
    selectedPresetId = presets[next].id;
    saveSettings();
  }

  function startCreatePreset() {
    presetEditorMode = "create";
    editingPreset = {
      id: "", name: "", description: "",
      target_height: 720, fps: 8, crf: 30,
      preset: "veryfast", audio_bitrate: 48000,
      audio_channels: 1, is_system: false,
      auto_transcribe: false,
      camera_overlay_enabled: false,
      camera_device_id: null,
      camera_overlay_size: "medium",
      camera_overlay_shape: "circle",
    };
    presetError = "";
    view = "preset-edit";
  }

  function startEditPreset(p) {
    presetEditorMode = "edit";
    editingPreset = { ...p };
    presetError = "";
    view = "preset-edit";
  }

  async function savePreset() {
    presetError = "";
    // Auto-set description from estimated size
    editingPreset.description = estimateFileSizePerHour(editingPreset);
    try {
      if (presetEditorMode === "create") {
        await invoke("cmd_create_preset", { preset: editingPreset });
      } else {
        await invoke("cmd_update_preset", { preset: editingPreset });
      }
      await loadPresets();
      view = "presets";
    } catch (e) {
      presetError = String(e);
    }
  }

  async function deletePreset(id) {
    try {
      await invoke("cmd_delete_preset", { id });
      await loadPresets();
      await loadSettings();
    } catch (e) {
      presetError = String(e);
    }
  }

  async function duplicatePreset(id) {
    try {
      await invoke("cmd_duplicate_preset", { id });
      await loadPresets();
    } catch (e) {
      presetError = String(e);
    }
  }

  async function resetPresetsToDefaults() {
    if (!confirm("Reset all presets to factory defaults? Custom presets will be deleted.")) return;
    try {
      presets = await invoke("cmd_reset_presets");
      await loadSettings();
      presetError = "";
    } catch (e) {
      presetError = String(e);
    }
  }

  // ── History ────────────────────────────────────────────────────
  async function loadHistory() {
    historyEntries = await invoke("get_history").catch(() => []);
  }

  async function deleteRecording(path) {
    try {
      await invoke("delete_recording_entry", { path });
      await loadHistory();
    } catch (e) { errorMsg = String(e); }
  }

  async function removeFromHistory(path) {
    await invoke("remove_from_history", { path });
    await loadHistory();
  }

  async function openFile(path) {
    try { await invoke("open_file", { path }); }
    catch (e) { errorMsg = String(e); }
  }

  async function openSrtViewer(path) {
    try {
      srtContent = await invoke("read_text_file", { path });
      srtViewerOpen = true;
    } catch (e) { errorMsg = String(e); }
  }

  async function revealFile(path) {
    try { await invoke("reveal_in_folder", { path }); }
    catch (e) { errorMsg = String(e); }
  }

  async function rescanHistory() {
    historyEntries = await invoke("rescan_history").catch(() => []);
  }

  // ── Transcription ─────────────────────────────────────────────
  async function checkWhisperModel() {
    whisperModelAvailable = await invoke("is_whisper_model_available").catch(() => false);
  }

  async function downloadModel() {
    modelDownloading = true;
    modelDownloadPercent = 0;
    showModelPrompt = false;
    try {
      await invoke("download_whisper_model");
      whisperModelAvailable = true;
    } catch (e) {
      errorMsg = "Model download failed: " + String(e);
    }
    modelDownloading = false;
  }

  async function loadTranscriptionQueue() {
    transcriptionJobs = await invoke("get_transcription_queue").catch(() => []);
    updateTranscriptionPolling();
  }

  function updateTranscriptionPolling() {
    const hasActive = transcriptionJobs.some(j => j.status === "pending" || j.status === "in_progress");
    if (hasActive && !transcriptionPollInterval) {
      transcriptionPollInterval = setInterval(async () => {
        transcriptionJobs = await invoke("get_transcription_queue").catch(() => []);
        historyEntries = await invoke("get_history").catch(() => historyEntries);
        const stillActive = transcriptionJobs.some(j => j.status === "pending" || j.status === "in_progress");
        if (!stillActive && transcriptionPollInterval) {
          clearInterval(transcriptionPollInterval);
          transcriptionPollInterval = null;
        }
      }, 2000);
    } else if (!hasActive && transcriptionPollInterval) {
      clearInterval(transcriptionPollInterval);
      transcriptionPollInterval = null;
    }
  }

  async function startTranscription(path) {
    try {
      // Check if Whisper model is available; download if not
      const modelReady = await invoke("is_whisper_model_available").catch(() => false);
      if (!modelReady) {
        // Start download and wait for it to finish
        await downloadModel();
        // Re-check after download
        const readyNow = await invoke("is_whisper_model_available").catch(() => false);
        if (!readyNow) {
          errorMsg = "Whisper model download failed. Please try again.";
          return;
        }
      }
      await invoke("transcribe_recording", { path });
      await loadTranscriptionQueue();
      await loadHistory();
    } catch (e) { errorMsg = String(e); }
  }

  async function retryTranscription(path) {
    try {
      const modelReady = await invoke("is_whisper_model_available").catch(() => false);
      if (!modelReady) {
        await downloadModel();
        const readyNow = await invoke("is_whisper_model_available").catch(() => false);
        if (!readyNow) {
          errorMsg = "Whisper model download failed. Please try again.";
          return;
        }
      }
      await invoke("retry_transcription", { path });
      await loadTranscriptionQueue();
      await loadHistory();
    } catch (e) { errorMsg = String(e); }
  }

  async function cancelTranscription(path) {
    try {
      await invoke("cancel_transcription", { path });
      await loadTranscriptionQueue();
      await loadHistory();
    } catch (e) { errorMsg = String(e); }
  }

  function getTranscriptionStatus(entry) {
    const job = transcriptionJobs.find(j => j.recording_path === entry.path);
    if (job) return { status: job.status, percent: job.progress_percent, error: job.error };
    if (entry.transcription_status) return { status: entry.transcription_status, percent: entry.transcription_status === "completed" ? 100 : 0 };
    return null;
  }

  async function forceClose() {
    showCloseWarning = false;
    await invoke("force_close_app");
  }

  // When toggling auto_transcribe on a preset, check model
  async function onAutoTranscribeToggle() {
    if (editingPreset.auto_transcribe && !whisperModelAvailable) {
      showModelPrompt = true;
    }
  }

  // ── Cleanup ────────────────────────────────────────────────────
  async function runCleanupNow() {
    const deleted = await invoke("run_cleanup").catch(() => 0);
    if (deleted > 0) {
      await loadHistory();
      errorMsg = `Cleaned up ${deleted} file(s)`;
    } else {
      errorMsg = "Nothing to clean up";
    }
    setTimeout(() => errorMsg = "", 3000);
  }

  // ── Lifecycle ──────────────────────────────────────────────────
  onMount(async () => {
    // Request screen recording permission via native API to register in System Settings
    try {
      const { checkScreenRecordingPermission, requestScreenRecordingPermission } = await import("tauri-plugin-macos-permissions-api");
      const hasScreenPerm = await checkScreenRecordingPermission();
      if (!hasScreenPerm) {
        await requestScreenRecordingPermission();
      }
    } catch (_) {}
    noPermission = !(await invoke("check_screen_permission"));
    devices = await invoke("list_audio_devices").catch(() => []);
    await loadPresets();
    await loadSettings();
    await loadHistory();

    if (!selectedDevice && devices.length > 0) selectedDevice = devices[0];
    if (await invoke("is_recording")) recording = true;

    await checkWhisperModel();
    await loadTranscriptionQueue();

    // Enumerate camera devices
    try {
      const allDevices = await navigator.mediaDevices.enumerateDevices();
      cameraDevices = allDevices.filter((d) => d.kind === "videoinput");
    } catch (_) {}

    unlisten.push(await listen("recording-status", (e) => {
      elapsed = e.payload.elapsed_seconds;
      fileSize = e.payload.file_size_bytes;
    }));
    unlisten.push(await listen("tray-start-recording", () => !recording && toggle()));
    unlisten.push(await listen("tray-stop-recording", () => recording && toggle()));
    unlisten.push(await listen("global-toggle-recording", toggle));

    // Transcription events
    unlisten.push(await listen("transcription-progress", (e) => {
      const { recording_path, percent } = e.payload;
      transcriptionJobs = transcriptionJobs.map(j =>
        j.recording_path === recording_path ? { ...j, progress_percent: percent, status: "in_progress" } : j
      );
    }));
    unlisten.push(await listen("transcription-completed", async () => {
      await loadTranscriptionQueue();
      await loadHistory();
    }));
    unlisten.push(await listen("transcription-failed", async () => {
      await loadTranscriptionQueue();
      await loadHistory();
    }));
    unlisten.push(await listen("transcription-started", async () => {
      await loadTranscriptionQueue();
    }));
    unlisten.push(await listen("model-download-progress", (e) => {
      modelDownloadPercent = e.payload.percent;
    }));
    unlisten.push(await listen("transcription-close-warning", () => {
      showCloseWarning = true;
    }));
  });

  onDestroy(() => {
    unlisten.forEach((u) => u());
    if (transcriptionPollInterval) clearInterval(transcriptionPollInterval);
  });

  // Debug: test camera overlay without recording
  async function toggleTestOverlay() {
    if (cameraOverlayOpen) {
      await invoke("close_camera_overlay");
      cameraOverlayOpen = false;
    } else {
      await invoke("open_camera_overlay", { deviceId: null, size: "medium", shape: "circle" });
      cameraOverlayOpen = true;
    }
  }

  // Derived
  let currentPreset = $derived(presets.find(p => p.id === selectedPresetId));
  let shortcutKey = $derived(
    typeof navigator !== "undefined" && navigator.platform?.includes("Mac") ? "\u2318" : "Ctrl"
  );
</script>

<div class="app" class:has-nav={view === "main"}>
  {#if noPermission}
    <div class="warn-bar">
      <div class="warn-icon">!</div>
      <div class="warn-text">
        <span class="warn-title">Screen Recording permission required</span>
        <span class="warn-sub">Toggle OFF then ON for Effective Recorder, then restart</span>
      </div>
      <button class="warn-action" onclick={grantAccess}>Open Settings</button>
    </div>
  {/if}

  <!-- ═══ MAIN VIEW ═══ -->
  {#if view === "main"}
    <div class="main-content">
      {#if recording}
        <!-- Recording state -->
        <div class="rec-indicator">
          <span class="rec-dot"></span>
          <span class="rec-label">REC</span>
        </div>
        <div class="timer">{fmt(elapsed)}</div>
        <div class="file-size">{fmtSize(fileSize)}</div>

        <button class="record-btn recording" onclick={toggle} disabled={loading}>
          <div class="btn-inner-stop">
            <div class="stop-icon"></div>
          </div>
        </button>
        <span class="btn-text-label">Stop</span>
      {:else}
        <!-- Ready / Saved state -->
        {#if savedPath}
          <div class="saved-msg">{savedPath.split("/").pop()}</div>
        {/if}

        {#if currentPreset}
          <button class="preset-bubble" onclick={cyclePreset} title="Click to switch preset">
            <span>{currentPreset.name}</span>
            <div class="bubble-arrow"></div>
          </button>
        {/if}

        <button class="record-btn" onclick={toggle} disabled={loading}>
          <div class="btn-inner">
            <span>Record</span>
          </div>
        </button>

        <div class="shortcut-hint">{shortcutKey} + Shift + R</div>
      {/if}

      {#if errorMsg}
        <div class="error-toast">{errorMsg}</div>
      {/if}
    </div>

    <nav class="tab-bar">
      <button class="tab" onclick={() => view = "settings"} disabled={recording}>
        <span class="tab-label">Settings</span>
      </button>
      <div class="tab-divider"></div>
      <button class="tab" onclick={() => { loadHistory(); view = "history"; }}>
        <span class="tab-label">History</span>
      </button>
      <div class="tab-divider"></div>
      <button class="tab" onclick={() => view = "presets"} disabled={recording}>
        <span class="tab-label">Presets</span>
      </button>
    </nav>

  <!-- ═══ SETTINGS VIEW ═══ -->
  {:else if view === "settings"}
    <div class="page">
      <header class="page-header">
        <button class="back" onclick={() => view = "main"}>
          <span class="back-arrow">&larr;</span> Back
        </button>
        <h2 class="page-title">Settings</h2>
      </header>

      <div class="page-body">
        <section class="section">
          <label class="section-label">Save to</label>
          <div class="field-row">
            <span class="field-value" title={outputDir || defaultOutputDir}>
              {shortDir(outputDir || defaultOutputDir)}
            </span>
            <div class="field-actions">
              <button class="pill-btn blue" onclick={pickFolder}>Change</button>
              {#if outputDir}
                <button class="pill-btn" onclick={resetFolder}>Reset</button>
              {/if}
            </div>
          </div>
        </section>

        <div class="divider"></div>

        <section class="section">
          <label class="section-label">Preset</label>
          <div class="select-wrap">
            <select bind:value={selectedPresetId} onchange={saveSettings}>
              {#each presets as p}
                <option value={p.id}>{p.name} ({estimateFileSizePerHour(p)})</option>
              {/each}
            </select>
          </div>
        </section>

        <div class="divider"></div>

        <section class="section">
          <label class="section-label">Microphone</label>
          <div class="select-wrap">
            <select bind:value={selectedDevice} onchange={saveSettings}>
              {#each devices as d}
                <option value={d}>{d}</option>
              {/each}
            </select>
          </div>
        </section>

        <div class="divider"></div>

        <button class="storage-btn" onclick={() => view = "storage"}>
          <span>Storage Policy</span>
          <span class="arrow-right">&rarr;</span>
        </button>

      </div>
    </div>

  <!-- ═══ STORAGE POLICY VIEW ═══ -->
  {:else if view === "storage"}
    <div class="page">
      <header class="page-header">
        <button class="back" onclick={() => view = "settings"}>
          <span class="back-arrow">&larr;</span> Back
        </button>
        <h2 class="page-title">Storage Policy</h2>
      </header>

      <div class="page-body">
        <section class="section">
          <label class="check-label">
            <input type="checkbox" bind:checked={storagePolicy.cleanup_enabled} onchange={saveSettings} />
            <span>Enable auto-cleanup</span>
          </label>
        </section>

        {#if storagePolicy.cleanup_enabled}
          <div class="divider"></div>
          <section class="section">
            <label class="section-label">Delete recordings older than</label>
            <div class="select-wrap">
              <select bind:value={storagePolicy.retention_days} onchange={saveSettings}>
                <option value={7}>7 days</option>
                <option value={30}>30 days</option>
                <option value={90}>90 days</option>
                <option value={180}>180 days</option>
                <option value={365}>365 days</option>
              </select>
            </div>
          </section>

          <div class="divider"></div>
          <section class="section">
            <label class="check-label">
              <input type="checkbox" bind:checked={storagePolicy.delete_large_old_files} onchange={saveSettings} />
              <span>Also delete large files older than 1 day</span>
            </label>
          </section>

          {#if storagePolicy.delete_large_old_files}
            <section class="section">
              <label class="section-label">Large file threshold (MB)</label>
              <input class="num-input" type="number" bind:value={storagePolicy.large_file_threshold_mb}
                min="50" step="50" onchange={saveSettings} />
            </section>
          {/if}

          <div class="divider"></div>
          <section class="section">
            <label class="section-label">Max files to delete per run</label>
            <input class="num-input" type="number" bind:value={storagePolicy.max_files_to_delete_per_run}
              min="1" max="100" onchange={saveSettings} />
          </section>
        {/if}

        <button class="action-btn" onclick={runCleanupNow}>Run cleanup now</button>

        {#if errorMsg}
          <div class="toast-info">{errorMsg}</div>
        {/if}
      </div>
    </div>

  <!-- ═══ HISTORY VIEW ═══ -->
  {:else if view === "history"}
    <div class="page">
      <header class="page-header">
        <button class="back" onclick={() => view = "main"}>
          <span class="back-arrow">&larr;</span> Back
        </button>
        <h2 class="page-title">Recording History</h2>
      </header>

      <div class="page-body">
        <div class="list-header">
          <span class="list-count">History ({historyEntries.length})</span>
          <button class="pill-btn" onclick={rescanHistory}>Rescan</button>
        </div>

        {#if historyEntries.length === 0}
          <div class="empty-state">No recordings yet</div>
        {:else}
          <div class="history-list">
            {#each historyEntries as entry, i}
              <div class="h-card" class:first={i === 0} class:missing={entry.status === "missing"}>
                <div class="h-card-top">
                  <span class="h-filename">{entry.filename}</span>
                  {#if entry.status === "missing"}
                    <span class="badge badge-red">Missing</span>
                  {/if}
                </div>
                <div class="h-meta">
                  <span>{fmtDate(entry.recorded_at)}</span>
                  <span class="h-sep">&middot;</span>
                  <span>{fmtDuration(entry.duration_seconds)}</span>
                  <span class="h-sep">&middot;</span>
                  <span>{fmtSize(entry.size_bytes)}</span>
                </div>
                {#if entry.transcription_status === "pending" || transcriptionJobs.some(j => j.recording_path === entry.path && j.status === "pending")}
                  <div class="transcription-row">
                    <span class="ts-badge ts-pending"><span class="ts-icon">&#9201;</span> Waiting...</span>
                  </div>
                {:else if entry.transcription_status === "in_progress" || transcriptionJobs.some(j => j.recording_path === entry.path && j.status === "in_progress")}
                  <div class="transcription-row">
                    <span class="ts-badge ts-progress">
                      <span class="ts-spinner"></span> Transcribing {transcriptionJobs.find(j => j.recording_path === entry.path)?.progress_percent || 0}%
                    </span>
                    <div class="ts-bar"><div class="ts-bar-fill" style="width: {transcriptionJobs.find(j => j.recording_path === entry.path)?.progress_percent || 0}%"></div></div>
                  </div>
                {:else if entry.transcription_status === "completed"}
                  <div class="transcription-row">
                    <span class="ts-badge ts-done">&#10003; Transcript ready</span>
                    {#if entry.transcript_path}
                      <button class="h-btn h-btn-small" onclick={() => openSrtViewer(entry.transcript_path)}>Open .srt</button>
                    {/if}
                  </div>
                {:else if entry.transcription_status === "failed" || transcriptionJobs.some(j => j.recording_path === entry.path && j.status === "failed")}
                  <div class="transcription-row">
                    <span class="ts-badge ts-fail">&#10007; Failed</span>
                    <button class="h-btn h-btn-small" onclick={() => retryTranscription(entry.path)}>Retry</button>
                    <button class="h-btn h-btn-small" onclick={() => cancelTranscription(entry.path)}>Dismiss</button>
                  </div>
                {:else if entry.status === "exists"}
                  <div class="transcription-row">
                    <button class="h-btn h-btn-small" onclick={() => startTranscription(entry.path)}>Transcribe</button>
                  </div>
                {/if}
                <div class="h-bottom">
                  <span class="preset-badge" style="background: {presetColor(entry.preset_name)}20; color: {presetColor(entry.preset_name)}; border: 1px solid {presetColor(entry.preset_name)}40">
                    {entry.preset_name}
                  </span>
                  <div class="h-actions">
                    {#if entry.status === "exists"}
                      <button class="h-btn" onclick={() => openFile(entry.path)}>
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/></svg>
                        Open
                      </button>
                      <button class="h-btn" onclick={() => revealFile(entry.path)}>
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
                        Reveal
                      </button>
                      <button class="h-btn h-btn-red" onclick={() => deleteRecording(entry.path)}>
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/></svg>
                        Delete
                      </button>
                    {:else}
                      <button class="h-btn" onclick={() => removeFromHistory(entry.path)}>Remove</button>
                    {/if}
                  </div>
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>

  <!-- ═══ PRESETS VIEW ═══ -->
  {:else if view === "presets"}
    <div class="page">
      <header class="page-header">
        <button class="back" onclick={() => view = "main"}>
          <span class="back-arrow">&larr;</span> Back
        </button>
        <h2 class="page-title">Presets</h2>
        <button class="pill-btn blue" onclick={startCreatePreset}>+ New</button>
      </header>

      <div class="page-body">
        {#if presetError}
          <div class="error-toast">{presetError}</div>
        {/if}

        <div class="preset-list">
          {#each presets as p}
            <div class="p-card" class:active={p.id === selectedPresetId}>
              <div class="p-card-top">
                <span class="p-name">{p.name}</span>
                <div class="p-tags">
                  {#if p.is_system}<span class="badge badge-gray">system</span>{/if}
                  {#if p.id === selectedPresetId}<span class="badge badge-green">active</span>{/if}
                </div>
              </div>
              <div class="p-specs">
                <span>{p.target_height}p</span>
                <span class="p-sep">&middot;</span>
                <span>{p.fps} fps</span>
                <span class="p-sep">&middot;</span>
                <span>CRF {p.crf}</span>
                <span class="p-sep">&middot;</span>
                <span>{p.preset}</span>
                <span class="p-sep">&middot;</span>
                <span>{p.audio_channels === 1 ? "mono" : "stereo"}</span>
              </div>
              <div class="p-desc">{estimateFileSizePerHour(p)}</div>
              <div class="p-actions">
                <button class="h-btn" onclick={() => { selectedPresetId = p.id; saveSettings(); }}>
                  {p.id === selectedPresetId ? "Active" : "Select"}
                </button>
                <button class="h-btn" onclick={() => startEditPreset(p)}>Edit</button>
                <button class="h-btn" onclick={() => duplicatePreset(p.id)}>Duplicate</button>
                {#if !p.is_system}
                  <button class="h-btn h-btn-red" onclick={() => deletePreset(p.id)}>Delete</button>
                {/if}
              </div>
            </div>
          {/each}
        </div>

        <div class="divider"></div>
        <button class="pill-btn" style="opacity: 0.6; font-size: 11px;" onclick={resetPresetsToDefaults}>
          Reset to defaults
        </button>
      </div>
    </div>

  {:else if view === "preset-edit"}
    <div class="page">
      <header class="page-header">
        <button class="back" onclick={() => view = "presets"}>
          <span class="back-arrow">&larr;</span> Back
        </button>
        <h2 class="page-title">{presetEditorMode === "create" ? "New Preset" : "Edit Preset"}</h2>
      </header>

      <div class="page-body">
        {#if presetError}
          <div class="error-toast">{presetError}</div>
        {/if}

        <section class="section">
          <label class="section-label">ID (unique, no spaces)</label>
          <input class="text-input" type="text" bind:value={editingPreset.id}
            disabled={presetEditorMode === "edit"} placeholder="my-preset" />
        </section>

        <section class="section">
          <label class="section-label">Name</label>
          <input class="text-input" type="text" bind:value={editingPreset.name} placeholder="My Preset" />
        </section>

        <div class="form-grid">
          <section class="section">
            <label class="section-label">Height (px)</label>
            <input class="num-input" type="number" bind:value={editingPreset.target_height} min="240" max="1080"
              onblur={() => clampPresetField('target_height', 240, 1080)} />
          </section>
          <section class="section">
            <label class="section-label">FPS</label>
            <input class="num-input" type="number" bind:value={editingPreset.fps} min="5" max="24"
              onblur={() => clampPresetField('fps', 5, 24)} />
          </section>
        </div>

        <div class="form-grid">
          <section class="section">
            <label class="section-label">CRF (0-51)</label>
            <input class="num-input" type="number" bind:value={editingPreset.crf} min="0" max="51"
              onblur={() => clampPresetField('crf', 0, 51)} />
          </section>
          <section class="section">
            <label class="section-label">x264 Preset</label>
            <div class="select-wrap">
              <select bind:value={editingPreset.preset}>
                {#each validEncoderPresets as ep}
                  <option value={ep}>{ep}</option>
                {/each}
              </select>
            </div>
          </section>
        </div>

        <div class="form-grid">
          <section class="section">
            <label class="section-label">Audio (bps)</label>
            <input class="num-input" type="number" bind:value={editingPreset.audio_bitrate} min="16000" max="320000" step="8000"
              onblur={() => clampPresetField('audio_bitrate', 16000, 320000)} />
          </section>
          <section class="section">
            <label class="section-label">Channels</label>
            <div class="select-wrap">
              <select bind:value={editingPreset.audio_channels}>
                <option value={1}>Mono</option>
                <option value={2}>Stereo</option>
              </select>
            </div>
          </section>
        </div>

        <section class="section">
          <label class="section-label">Estimated size</label>
          <input class="text-input" type="text" value={estimateFileSizePerHour(editingPreset)} disabled
            style="background: var(--bg-tertiary); opacity: 0.8;" />
        </section>

        <div class="divider"></div>
        <section class="section">
          <label class="check-label">
            <input type="checkbox" bind:checked={editingPreset.auto_transcribe} onchange={onAutoTranscribeToggle} />
            <span>Auto-transcribe after recording</span>
          </label>
          <span class="field-hint">Uses local Whisper AI to generate subtitles (.srt)</span>
        </section>

        <div class="divider"></div>
        <section class="section">
          <label class="check-label">
            <input type="checkbox" bind:checked={editingPreset.camera_overlay_enabled} />
            <span>Camera overlay during recording</span>
          </label>
          <span class="field-hint">Shows a floating webcam bubble on screen (Loom-style)</span>
        </section>

        {#if editingPreset.camera_overlay_enabled}
          <section class="section">
            <label class="section-label">Camera</label>
            <div class="select-wrap">
              <select bind:value={editingPreset.camera_device_id}>
                <option value={null}>Default camera</option>
                {#each cameraDevices as cam}
                  <option value={cam.deviceId}>{cam.label || `Camera ${cam.deviceId.slice(0, 8)}`}</option>
                {/each}
              </select>
            </div>
          </section>

          <div class="form-grid">
            <section class="section">
              <label class="section-label">Size</label>
              <div class="select-wrap">
                <select bind:value={editingPreset.camera_overlay_size}>
                  <option value="small">Small (150px)</option>
                  <option value="medium">Medium (200px)</option>
                  <option value="large">Large (300px)</option>
                </select>
              </div>
            </section>
            <section class="section">
              <label class="section-label">Shape</label>
              <div class="select-wrap">
                <select bind:value={editingPreset.camera_overlay_shape}>
                  <option value="circle">Circle</option>
                  <option value="rounded">Rounded</option>
                </select>
              </div>
            </section>
          </div>
        {/if}

        <button class="action-btn" onclick={savePreset}>
          {presetEditorMode === "create" ? "Create Preset" : "Save Changes"}
        </button>
      </div>
    </div>
  {/if}

  <!-- ═══ MODEL DOWNLOAD OVERLAY ═══ -->
  {#if showModelPrompt}
    <div class="overlay">
      <div class="modal">
        <h3 class="modal-title">Whisper Model Required</h3>
        <p class="modal-text">Auto-transcription needs the Whisper large-v3-turbo model (~1.5 GB). Download it now?</p>
        <div class="modal-actions">
          <button class="modal-btn" onclick={() => showModelPrompt = false}>Cancel</button>
          <button class="modal-btn modal-btn-primary" onclick={downloadModel}>Download</button>
        </div>
      </div>
    </div>
  {/if}

  <!-- ═══ MODEL DOWNLOADING OVERLAY ═══ -->
  {#if modelDownloading}
    <div class="overlay">
      <div class="modal">
        <h3 class="modal-title">Downloading Whisper Model</h3>
        <p class="modal-text">ggml-large-v3-turbo.bin — {modelDownloadPercent}%</p>
        <div class="ts-bar modal-bar"><div class="ts-bar-fill" style="width: {modelDownloadPercent}%"></div></div>
      </div>
    </div>
  {/if}

  <!-- ═══ CLOSE WARNING OVERLAY ═══ -->
  {#if showCloseWarning}
    <div class="overlay">
      <div class="modal">
        <h3 class="modal-title">Transcription in Progress</h3>
        <p class="modal-text">A recording is being transcribed. If you close now, it will resume on next launch.</p>
        <div class="modal-actions">
          <button class="modal-btn modal-btn-primary" onclick={() => showCloseWarning = false}>Wait</button>
          <button class="modal-btn modal-btn-danger" onclick={forceClose}>Close Anyway</button>
        </div>
      </div>
    </div>
  {/if}

  {#if srtViewerOpen}
    <div class="overlay" onclick={() => srtViewerOpen = false}>
      <div class="srt-modal" onclick={(e) => e.stopPropagation()}>
        <div class="srt-header">
          <h3 class="modal-title">Transcript</h3>
          <div class="srt-header-actions">
            <button class="srt-copy" onclick={async () => {
              await navigator.clipboard.writeText(srtContent);
              const btn = document.querySelector('.srt-copy');
              btn.textContent = 'Copied!';
              setTimeout(() => btn.textContent = 'Copy', 1500);
            }}>Copy</button>
            <button class="srt-close" onclick={() => srtViewerOpen = false}>&times;</button>
          </div>
        </div>
        <pre class="srt-content">{srtContent}</pre>
      </div>
    </div>
  {/if}
</div>

<style>
  /* ── Reset & Base ─────────────────────────────────────────── */
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Display", "Segoe UI", system-ui, sans-serif;
    background: #0f1623;
    color: #e2e8f0;
    -webkit-user-select: none;
    user-select: none;
    overflow: hidden;
  }

  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100vw;
    overflow: hidden;
  }

  /* ── Permission Banner ───────────────────────────────────── */
  .warn-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    background: rgba(245, 158, 11, 0.08);
    border-bottom: 1px solid rgba(245, 158, 11, 0.15);
    flex-shrink: 0;
  }
  .warn-icon {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: rgba(245, 158, 11, 0.2);
    color: #f59e0b;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    font-weight: 700;
    flex-shrink: 0;
  }
  .warn-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .warn-title { font-size: 11px; color: #fbbf24; font-weight: 600; }
  .warn-sub { font-size: 10px; color: #b48a1a; }
  .warn-action {
    background: none;
    border: 1px solid rgba(245, 158, 11, 0.3);
    color: #fbbf24;
    font-size: 10px;
    padding: 4px 10px;
    border-radius: 6px;
    cursor: pointer;
    white-space: nowrap;
  }
  .warn-action:hover { background: rgba(245, 158, 11, 0.1); }

  /* ── Main View ───────────────────────────────────────────── */
  .main-content {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 20px;
    padding-bottom: 0;
  }

  /* Recording indicator */
  .rec-indicator {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 4px;
  }
  .rec-dot {
    width: 10px;
    height: 10px;
    background: #ef4444;
    border-radius: 50%;
    animation: pulse 1.2s ease-in-out infinite;
    box-shadow: 0 0 8px rgba(239, 68, 68, 0.6);
  }
  @keyframes pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.3; transform: scale(0.85); }
  }
  .rec-label {
    font-size: 14px;
    font-weight: 700;
    color: #ef4444;
    letter-spacing: 3px;
  }

  /* Timer */
  .timer {
    font-size: 56px;
    font-weight: 200;
    color: #ffffff;
    font-variant-numeric: tabular-nums;
    line-height: 1;
    letter-spacing: -1px;
  }
  .file-size {
    font-size: 14px;
    color: #64748b;
    margin-bottom: 16px;
  }

  /* Record button */
  .record-btn {
    width: 130px;
    height: 130px;
    border-radius: 50%;
    border: none;
    cursor: pointer;
    position: relative;
    background: transparent;
    padding: 0;
    transition: transform 0.15s;
    outline: none;
  }
  .record-btn:hover:not(:disabled) { transform: scale(1.03); }
  .record-btn:active:not(:disabled) { transform: scale(0.97); }
  .record-btn:disabled { opacity: 0.5; cursor: wait; }

  .btn-inner {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    background: radial-gradient(circle at 40% 35%, #1a4a3a, #0d2e24 70%);
    border: 2px solid rgba(52, 211, 153, 0.25);
    box-shadow:
      0 0 30px rgba(16, 185, 129, 0.15),
      0 0 60px rgba(16, 185, 129, 0.08),
      inset 0 1px 0 rgba(255, 255, 255, 0.05);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 18px;
    font-weight: 500;
    color: #d1fae5;
    letter-spacing: 0.5px;
  }

  .btn-inner-stop {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    background: radial-gradient(circle at 40% 35%, #7f1d1d, #450a0a 70%);
    border: 2px solid rgba(239, 68, 68, 0.35);
    box-shadow:
      0 0 30px rgba(239, 68, 68, 0.2),
      0 0 60px rgba(239, 68, 68, 0.1),
      inset 0 1px 0 rgba(255, 255, 255, 0.05);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .stop-icon {
    width: 24px;
    height: 24px;
    border-radius: 4px;
    background: #fca5a5;
  }
  .btn-text-label {
    font-size: 14px;
    color: #9ca3af;
    margin-top: 8px;
  }

  /* Preset bubble */
  .preset-bubble {
    position: relative;
    background: rgba(30, 58, 95, 0.7);
    color: #93c5fd;
    font-size: 12px;
    font-weight: 500;
    padding: 5px 14px;
    border-radius: 8px;
    border: 1px solid rgba(59, 130, 246, 0.2);
    margin-bottom: 8px;
    cursor: pointer;
    transition: background 0.15s;
  }
  .preset-bubble:hover {
    background: rgba(30, 58, 95, 0.95);
  }
  .bubble-arrow {
    position: absolute;
    bottom: -6px;
    left: 50%;
    transform: translateX(-50%);
    width: 0;
    height: 0;
    border-left: 6px solid transparent;
    border-right: 6px solid transparent;
    border-top: 6px solid rgba(30, 58, 95, 0.7);
  }

  /* Shortcut hint */
  .shortcut-hint {
    font-size: 13px;
    color: #374151;
    margin-top: 12px;
    letter-spacing: 1px;
  }

  /* Saved message */
  .saved-msg {
    font-size: 12px;
    color: #34d399;
    margin-bottom: 8px;
    text-align: center;
    word-break: break-all;
    max-width: 260px;
  }

  /* Error toast */
  .error-toast {
    font-size: 11px;
    color: #fca5a5;
    background: rgba(239, 68, 68, 0.1);
    border: 1px solid rgba(239, 68, 68, 0.15);
    padding: 8px 14px;
    border-radius: 8px;
    text-align: center;
    word-break: break-word;
    max-width: 100%;
    margin-top: 8px;
  }

  /* ── Tab Bar ─────────────────────────────────────────────── */
  .tab-bar {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 14px 20px;
    border-top: 1px solid #1e293b;
    flex-shrink: 0;
    gap: 0;
  }
  .tab {
    flex: 1;
    background: none;
    border: none;
    color: #94a3b8;
    font-size: 13px;
    font-weight: 500;
    padding: 4px 0;
    cursor: pointer;
    text-align: center;
    transition: color 0.15s;
  }
  .tab:hover:not(:disabled) { color: #e2e8f0; }
  .tab:disabled { color: #334155; cursor: default; }
  .tab-divider {
    width: 1px;
    height: 16px;
    background: #1e293b;
    flex-shrink: 0;
  }

  /* ── Page Layout (Settings, History, Presets) ─────────────── */
  .page {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
  }
  .page-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 16px 20px;
    flex-shrink: 0;
  }
  .page-title {
    flex: 1;
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    color: #f1f5f9;
  }
  .back {
    background: none;
    border: none;
    color: #60a5fa;
    font-size: 13px;
    cursor: pointer;
    padding: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }
  .back:hover { color: #93c5fd; }
  .back-arrow { font-size: 15px; }

  .page-body {
    flex: 1;
    overflow-y: auto;
    padding: 0 20px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  /* ── Sections & Fields ───────────────────────────────────── */
  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .section-label {
    font-size: 11px;
    font-weight: 600;
    color: #64748b;
    text-transform: uppercase;
    letter-spacing: 0.8px;
  }
  .divider {
    height: 1px;
    background: #1e293b;
    margin: 4px 0;
  }

  .field-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .field-value {
    flex: 1;
    font-size: 14px;
    color: #cbd5e1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .field-actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  /* Select */
  .select-wrap {
    position: relative;
  }
  .select-wrap select,
  .page-body select {
    width: 100%;
    padding: 10px 14px;
    background: #1a2332;
    color: #cbd5e1;
    border: 1px solid #2d3b4e;
    border-radius: 10px;
    font-size: 13px;
    outline: none;
    appearance: none;
    -webkit-appearance: none;
    cursor: pointer;
    box-sizing: border-box;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%2364748b' stroke-width='2'%3E%3Cpath d='m6 9 6 6 6-6'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 12px center;
    padding-right: 32px;
  }
  .select-wrap select:focus { border-color: #3b82f6; }

  /* Inputs */
  .text-input, .num-input {
    width: 100%;
    padding: 10px 14px;
    background: #1a2332;
    color: #cbd5e1;
    border: 1px solid #2d3b4e;
    border-radius: 10px;
    font-size: 13px;
    outline: none;
    box-sizing: border-box;
  }
  .text-input:focus, .num-input:focus { border-color: #3b82f6; }
  .text-input:disabled, .num-input:disabled { opacity: 0.5; }

  /* Checkbox */
  .check-label {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 13px;
    color: #cbd5e1;
    cursor: pointer;
  }
  .check-label input[type="checkbox"] {
    width: 18px;
    height: 18px;
    accent-color: #3b82f6;
    flex-shrink: 0;
  }

  .form-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }

  /* ── Pill Buttons ────────────────────────────────────────── */
  .pill-btn {
    background: #1e293b;
    border: 1px solid #334155;
    color: #94a3b8;
    padding: 6px 14px;
    border-radius: 8px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s;
  }
  .pill-btn:hover { background: #334155; color: #e2e8f0; }
  .pill-btn.blue {
    background: rgba(59, 130, 246, 0.15);
    border-color: rgba(59, 130, 246, 0.3);
    color: #60a5fa;
  }
  .pill-btn.blue:hover { background: rgba(59, 130, 246, 0.25); }

  /* Storage Policy button */
  .storage-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    width: 100%;
    padding: 12px;
    background: #1a2332;
    border: 1px solid #2d3b4e;
    border-radius: 10px;
    color: #94a3b8;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.15s;
    margin-top: 4px;
  }
  .storage-btn:hover { background: #1e293b; color: #cbd5e1; border-color: #3b82f6; }
  .arrow-right { font-size: 16px; }

  /* Action button (cleanup, save) */
  .action-btn {
    width: 100%;
    padding: 12px;
    background: rgba(59, 130, 246, 0.15);
    border: 1px solid rgba(59, 130, 246, 0.25);
    border-radius: 10px;
    color: #60a5fa;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
    margin-top: 8px;
  }
  .action-btn:hover { background: rgba(59, 130, 246, 0.25); }

  .toast-info {
    font-size: 12px;
    color: #60a5fa;
    text-align: center;
    padding: 6px;
  }

  /* ── History List ────────────────────────────────────────── */
  .list-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 4px;
  }
  .list-count {
    font-size: 13px;
    color: #94a3b8;
    font-weight: 500;
  }

  .empty-state {
    font-size: 14px;
    color: #475569;
    text-align: center;
    padding: 40px 0;
  }

  .history-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .h-card {
    background: #151e2d;
    border: 1px solid #1e293b;
    border-radius: 12px;
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    transition: border-color 0.15s;
  }
  .h-card.first {
    border-color: rgba(59, 130, 246, 0.3);
    background: #161f30;
  }
  .h-card.missing { opacity: 0.5; }

  .h-card-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .h-filename {
    flex: 1;
    font-size: 13px;
    font-weight: 500;
    color: #e2e8f0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .h-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: #64748b;
  }
  .h-sep { color: #334155; }

  .h-bottom {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 2px;
  }

  .preset-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 3px 10px;
    border-radius: 6px;
    white-space: nowrap;
  }

  .h-actions {
    display: flex;
    gap: 4px;
  }

  .h-btn {
    display: flex;
    align-items: center;
    gap: 4px;
    background: #1e293b;
    border: 1px solid #334155;
    color: #94a3b8;
    padding: 5px 10px;
    border-radius: 6px;
    font-size: 11px;
    cursor: pointer;
    white-space: nowrap;
    transition: all 0.15s;
  }
  .h-btn:hover { background: #334155; color: #e2e8f0; }
  .h-btn-red { color: #f87171; border-color: rgba(239, 68, 68, 0.2); }
  .h-btn-red:hover { background: rgba(239, 68, 68, 0.15); color: #fca5a5; }

  .h-btn svg { flex-shrink: 0; }

  /* ── Badges ──────────────────────────────────────────────── */
  .badge {
    font-size: 10px;
    font-weight: 600;
    padding: 2px 8px;
    border-radius: 6px;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    white-space: nowrap;
  }
  .badge-gray { background: #334155; color: #94a3b8; }
  .badge-green { background: rgba(52, 211, 153, 0.15); color: #34d399; border: 1px solid rgba(52, 211, 153, 0.2); }
  .badge-red { background: rgba(239, 68, 68, 0.15); color: #f87171; border: 1px solid rgba(239, 68, 68, 0.2); }

  /* ── Presets List ────────────────────────────────────────── */
  .preset-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .p-card {
    background: #151e2d;
    border: 1px solid #1e293b;
    border-radius: 12px;
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    transition: border-color 0.15s;
  }
  .p-card.active {
    border-color: rgba(59, 130, 246, 0.3);
    background: #161f30;
  }

  .p-card-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .p-name {
    flex: 1;
    font-size: 14px;
    font-weight: 600;
    color: #e2e8f0;
  }
  .p-tags {
    display: flex;
    gap: 4px;
  }

  .p-specs {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    font-size: 12px;
    color: #64748b;
  }
  .p-sep { color: #334155; }

  .p-desc {
    font-size: 12px;
    color: #475569;
    font-style: italic;
  }

  .p-actions {
    display: flex;
    gap: 4px;
    margin-top: 4px;
  }

  /* ── Transcription Status ────────────────────────────────── */
  .transcription-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .ts-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 3px 10px;
    border-radius: 6px;
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .ts-pending {
    background: rgba(245, 158, 11, 0.1);
    color: #f59e0b;
    border: 1px solid rgba(245, 158, 11, 0.2);
  }
  .ts-progress {
    background: rgba(59, 130, 246, 0.1);
    color: #60a5fa;
    border: 1px solid rgba(59, 130, 246, 0.2);
  }
  .ts-done {
    background: rgba(52, 211, 153, 0.1);
    color: #34d399;
    border: 1px solid rgba(52, 211, 153, 0.2);
  }
  .ts-fail {
    background: rgba(239, 68, 68, 0.1);
    color: #f87171;
    border: 1px solid rgba(239, 68, 68, 0.2);
  }
  .ts-icon { font-size: 12px; }

  .ts-spinner {
    width: 10px;
    height: 10px;
    border: 2px solid rgba(96, 165, 250, 0.3);
    border-top-color: #60a5fa;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .ts-bar {
    flex: 1;
    min-width: 60px;
    height: 4px;
    background: #1e293b;
    border-radius: 2px;
    overflow: hidden;
  }
  .ts-bar-fill {
    height: 100%;
    background: #3b82f6;
    border-radius: 2px;
    transition: width 0.3s;
  }

  .h-btn-small {
    padding: 3px 8px !important;
    font-size: 10px !important;
  }

  .field-hint {
    font-size: 11px;
    color: #475569;
    margin-top: -2px;
  }

  /* ── Overlay / Modal ────────────────────────────────────── */
  .overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
    backdrop-filter: blur(4px);
  }
  .modal {
    background: #1a2332;
    border: 1px solid #2d3b4e;
    border-radius: 16px;
    padding: 24px;
    width: 300px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .modal-title {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: #f1f5f9;
  }
  .modal-text {
    margin: 0;
    font-size: 13px;
    color: #94a3b8;
    line-height: 1.5;
  }
  .modal-actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
    margin-top: 4px;
  }
  .modal-btn {
    padding: 8px 18px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid #334155;
    background: #1e293b;
    color: #94a3b8;
    transition: all 0.15s;
  }
  .modal-btn:hover { background: #334155; color: #e2e8f0; }
  .modal-btn-primary {
    background: rgba(59, 130, 246, 0.2);
    border-color: rgba(59, 130, 246, 0.3);
    color: #60a5fa;
  }
  .modal-btn-primary:hover { background: rgba(59, 130, 246, 0.3); }
  .modal-btn-danger {
    background: rgba(239, 68, 68, 0.15);
    border-color: rgba(239, 68, 68, 0.25);
    color: #f87171;
  }
  .modal-btn-danger:hover { background: rgba(239, 68, 68, 0.25); }
  .modal-bar {
    margin-top: 4px;
    height: 6px;
  }

  /* ── Scrollbar ───────────────────────────────────────────── */
  .page-body::-webkit-scrollbar { width: 4px; }
  .page-body::-webkit-scrollbar-track { background: transparent; }
  .page-body::-webkit-scrollbar-thumb { background: #1e293b; border-radius: 2px; }
  .page-body::-webkit-scrollbar-thumb:hover { background: #334155; }
  /* ── SRT Viewer ──────────────────────────────────────────── */
  .srt-modal {
    background: #1a2332;
    border: 1px solid #2d3b4e;
    border-radius: 16px;
    padding: 20px;
    width: 340px;
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .srt-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .srt-header-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .srt-copy {
    background: #334155;
    border: 1px solid #475569;
    color: #cbd5e1;
    font-size: 12px;
    cursor: pointer;
    padding: 4px 10px;
    border-radius: 6px;
    transition: all 0.15s;
  }
  .srt-copy:hover { background: #475569; color: #f1f5f9; }
  .srt-close {
    background: none;
    border: none;
    color: #94a3b8;
    font-size: 22px;
    cursor: pointer;
    padding: 0 4px;
  }
  .srt-close:hover { color: #f1f5f9; }
  .srt-content {
    margin: 0;
    padding: 12px;
    background: #0f1923;
    border-radius: 8px;
    color: #cbd5e1;
    font-size: 12px;
    line-height: 1.5;
    overflow-y: auto;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 55vh;
  }

</style>
