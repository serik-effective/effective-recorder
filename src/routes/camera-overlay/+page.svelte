<script>
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, onDestroy } from "svelte";
  import { checkCameraPermission, requestCameraPermission } from "tauri-plugin-macos-permissions-api";

  let videoEl = $state(null);
  let stream = $state(null);
  let error = $state(false);

  const params = new URLSearchParams(window.location.search);
  const deviceId = params.get("deviceId");
  const size = params.get("size") || "medium";
  const shape = params.get("shape") || "circle";

  const dimensions = { small: 150, medium: 200, large: 300 };
  const dim = dimensions[size] || 200;
  const radius = shape === "circle" ? "50%" : "16px";

  async function log(msg) {
    console.log(msg);
    try { await invoke("log_from_js", { message: msg }); } catch(_) {}
  }

  onMount(async () => {
    try {
      await log("Camera overlay mounted, checking permission...");
      let hasPerm = await checkCameraPermission();
      await log(`Camera permission check: ${hasPerm}`);
      if (!hasPerm) {
        await requestCameraPermission();
        const maxWait = 30_000;
        const interval = 500;
        let waited = 0;
        while (waited < maxWait) {
          await new Promise(r => setTimeout(r, interval));
          waited += interval;
          hasPerm = await checkCameraPermission();
          if (hasPerm) break;
        }
        await log(`Camera permission after wait: ${hasPerm}`);
        if (!hasPerm) {
          await log("Camera permission denied or timed out");
          error = true;
          return;
        }
      }

      // Check available devices
      const devices = await navigator.mediaDevices.enumerateDevices();
      const cameras = devices.filter(d => d.kind === "videoinput");
      await log(`Found ${cameras.length} cameras: ${cameras.map(c => c.label || c.deviceId).join(", ")}`);

      const constraints = {
        video: deviceId
          ? { deviceId: { exact: deviceId }, width: { ideal: dim * 2 }, height: { ideal: dim * 2 } }
          : { width: { ideal: dim * 2 }, height: { ideal: dim * 2 } },
        audio: false,
      };
      await log(`getUserMedia with: ${JSON.stringify(constraints)}`);
      stream = await navigator.mediaDevices.getUserMedia(constraints);
      await log(`getUserMedia success, tracks: ${stream.getTracks().map(t => t.label).join(", ")}`);
      if (videoEl) {
        videoEl.srcObject = stream;
      }
    } catch (e) {
      await log(`Camera error: ${e?.name} - ${e?.message}`);
      if (deviceId) {
        try {
          await log("Retrying without specific deviceId...");
          stream = await navigator.mediaDevices.getUserMedia({ video: { width: { ideal: dim * 2 }, height: { ideal: dim * 2 } }, audio: false });
          if (videoEl) { videoEl.srcObject = stream; }
          return;
        } catch (e2) {
          await log(`Camera fallback failed: ${e2?.name} - ${e2?.message}`);
        }
      }
      error = true;
    }
  });

  onDestroy(() => {
    if (stream) {
      stream.getTracks().forEach((t) => t.stop());
    }
  });

  function startDrag() {
    getCurrentWindow().startDragging();
  }

  function close() {
    invoke("save_camera_position").then(() => {
      invoke("close_camera_overlay");
    });
  }
</script>

<div
  class="overlay"
  data-tauri-drag-region
  role="presentation"
  onmousedown={startDrag}
  style="width: {dim}px; height: {dim}px; border-radius: {radius};"
>
  {#if error}
    <div class="error">
      <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="#ff6b6b" stroke-width="2">
        <path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z" />
        <line x1="9" y1="13" x2="15" y2="13" />
      </svg>
      <span>No camera</span>
    </div>
  {:else}
    <video
      bind:this={videoEl}
      autoplay
      muted
      playsinline
      style="width: {dim}px; height: {dim}px; border-radius: {radius};"
    ></video>
  {/if}

  <button class="close-btn" onmousedown={(e) => e.stopPropagation()} onclick={close} title="Close camera overlay">
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="white" stroke-width="2">
      <line x1="1" y1="1" x2="9" y2="9" />
      <line x1="9" y1="1" x2="1" y2="9" />
    </svg>
  </button>
</div>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    padding: 0;
    background: transparent;
    overflow: hidden;
  }

  .overlay {
    position: fixed;
    top: 0;
    left: 0;
    overflow: hidden;
    background: transparent;
    border: 2px solid rgba(255, 255, 255, 0.2);
    cursor: grab;
    user-select: none;
  }

  .overlay:active {
    cursor: grabbing;
  }

  video {
    display: block;
    object-fit: cover;
    transform: scaleX(-1);
    pointer-events: none;
  }

  .error {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 8px;
    color: #ff6b6b;
    font-family: -apple-system, BlinkMacSystemFont, sans-serif;
    font-size: 12px;
  }

  .close-btn {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: none;
    background: rgba(0, 0, 0, 0.6);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transition: opacity 0.2s;
    padding: 0;
    z-index: 10;
  }

  .overlay:hover .close-btn {
    opacity: 1;
  }

  .close-btn:hover {
    background: rgba(255, 60, 60, 0.8);
  }
</style>
