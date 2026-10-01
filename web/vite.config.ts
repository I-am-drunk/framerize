import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Single-origin dev: proxy the API to the Rust daemon. Never bake an origin into the
// bundle — remote browsers must reach the API through the same host that served them.
export default defineConfig({
  plugins: [react()],
  build: { outDir: "dist" },
  server: {
    proxy: { "/api": "http://127.0.0.1:8787" },
  },
});
