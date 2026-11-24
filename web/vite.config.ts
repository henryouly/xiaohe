import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "vite";
import solidPlugin from "vite-plugin-solid";

export default defineConfig({
  plugins: [solidPlugin(), tailwindcss()],
  server: {
    port: 5173,
    proxy: {
      "/api": "http://127.0.0.1:4000",
      "/health": "http://127.0.0.1:4000",
      "/docs": "http://127.0.0.1:4000",
    },
  },
  build: {
    target: "esnext",
    outDir: "dist",
    sourcemap: false,
  },
});
