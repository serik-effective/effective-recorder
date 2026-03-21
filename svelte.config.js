// Tauri doesn't have a Node.js server to do proper SSR
// so we use adapter-static with a fallback to index.html to put the site in SPA mode
// See: https://svelte.dev/docs/kit/single-page-apps
// See: https://v2.tauri.app/start/frontend/sveltekit/ for more info
import adapter from "@sveltejs/adapter-static";

/** @type {import('@sveltejs/kit').Config} */
const config = {
  compilerOptions: {
    warningFilter: (warning) => {
      // Suppress a11y warnings for desktop app (not a web page)
      if (warning.code.startsWith("a11y")) return false;
      return true;
    },
  },
  kit: {
    adapter: adapter({
      fallback: "index.html",
    }),
  },
};

export default config;
